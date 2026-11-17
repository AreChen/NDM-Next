use ndm_core::{Store, Task, TaskId, TaskState};

/// A new transfer gets a new identity and probes the resource again. The old
/// task and its output stay intact; the core publishes with collision-safe names.
pub(crate) fn enqueue(store: &Store, source: TaskId) -> ndm_core::Result<Task> {
    let task = store.get(source)?.ok_or(ndm_core::Error::NotFound)?;
    if task.state == TaskState::Prepared {
        return Err(ndm_core::Error::InvalidState);
    }
    let mut request = task.request;
    request.expected_size = None;
    if request.suggested_filename.is_none() {
        request.suggested_filename = task
            .output_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned());
    }
    let id = TaskId::new_v4();
    store.prepare_with_options(id, &request, &task.options)?;
    match store.commit(id) {
        Ok(task) => Ok(task),
        Err(error) => {
            let _ = store.abort(id);
            Err(error)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndm_core::{HlsOptions, NewTask, TaskOptions};
    use std::collections::BTreeMap;

    #[test]
    fn redownload_keeps_media_choices_and_credentials_but_not_old_progress_or_size() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(&dir.path().join("tasks.db")).unwrap();
        let id = TaskId::new_v4();
        let request = NewTask {
            url: "https://example.test/stream.m3u8".into(),
            destination: dir.path().into(),
            suggested_filename: Some("video.mp4".into()),
            source_url: Some("https://example.test/watch".into()),
            expected_size: Some(900),
            headers: BTreeMap::from([("Authorization".into(), "fixture-token".into())]),
        };
        let options = TaskOptions::Hls(HlsOptions {
            variant_index: Some(2),
            audio_names: vec!["English".into()],
            ..Default::default()
        });
        store.prepare_with_options(id, &request, &options).unwrap();
        assert!(enqueue(&store, id).is_err());
        store.commit(id).unwrap();
        store.pause(id).unwrap();
        let new = enqueue(&store, id).unwrap();
        assert_ne!(new.id, id);
        assert_eq!(new.options, options);
        let mut expected = request.clone();
        expected.expected_size = None;
        assert_eq!(new.request, expected);
        assert_eq!(new.state, TaskState::Queued);
        assert!(new.progress.is_none());
        assert!(new.output_path.is_none());
        let old = store.get(id).unwrap().unwrap();
        assert_eq!(old.state, TaskState::Paused);
        assert_eq!(old.request, request);
    }

    #[tokio::test]
    async fn completed_task_downloads_to_a_new_file_without_overwriting_the_original() {
        use ndm_core::{Scheduler, SchedulerOptions};
        use std::{sync::Arc, time::Duration};
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut request = [0; 4096];
                let _ = stream.read(&mut request).await;
                let _ = stream
                    .write_all(
                        b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndata",
                    )
                    .await;
            }
        });
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
        let scheduler = Scheduler::start(
            store.clone(),
            SchedulerOptions {
                auto_resume: false,
                ..Default::default()
            },
        )
        .unwrap();
        let id = TaskId::new_v4();
        store
            .prepare(
                id,
                &NewTask {
                    url: format!("http://{address}/file.bin"),
                    destination: dir.path().into(),
                    suggested_filename: None,
                    source_url: None,
                    expected_size: None,
                    headers: BTreeMap::new(),
                },
            )
            .unwrap();
        store.commit(id).unwrap();
        scheduler.notify();
        async fn completed(store: &Store, id: TaskId) -> Task {
            tokio::time::timeout(Duration::from_secs(5), async {
                loop {
                    if let Some(task) = store.get(id).unwrap()
                        && task.state == TaskState::Completed
                    {
                        break task;
                    }
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap()
        }
        let original = completed(&store, id).await;
        let repeated = enqueue(&store, id).unwrap();
        scheduler.notify();
        let repeated = completed(&store, repeated.id).await;
        assert_ne!(original.output_path, repeated.output_path);
        for path in [original.output_path, repeated.output_path] {
            assert_eq!(std::fs::read(path.unwrap()).unwrap(), b"data");
        }
        assert_eq!(store.get(id).unwrap().unwrap().state, TaskState::Completed);
        scheduler.shutdown().await;
        server.abort();
    }
}
