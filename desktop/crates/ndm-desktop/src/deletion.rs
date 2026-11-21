use ndm_core::{Scheduler, Store, TaskId, TaskState};
use std::sync::Arc;

#[derive(Default)]
pub(crate) struct DeleteReport {
    pub removed: Vec<TaskId>,
    pub failed: Vec<TaskId>,
}

/// Stop active workers before touching their files. A failed file operation keeps its record.
pub(crate) async fn delete_many(
    store: Arc<Store>,
    scheduler: Scheduler,
    ids: Vec<TaskId>,
    delete_files: bool,
) -> DeleteReport {
    let mut report = DeleteReport::default();
    for id in ids {
        let initial = store.get(id).ok().flatten();
        if initial
            .as_ref()
            .is_some_and(|task| matches!(task.state, TaskState::Running | TaskState::Queued))
        {
            // Completion can race with a pause; re-read the final state below.
            let _ = scheduler.pause(id).await;
        }
        let Some(task) = store.get(id).ok().flatten() else {
            report.failed.push(id);
            continue;
        };
        if matches!(task.state, TaskState::Prepared | TaskState::Running) {
            report.failed.push(id);
            continue;
        }
        scheduler.stop_sharing(id).await;
        let scheduler = scheduler.clone();
        let removed = tokio::task::spawn_blocking(move || {
            scheduler.delete_with(id, delete_files, |task| {
                if delete_files && task.state == TaskState::Completed {
                    crate::files::delete_output(task)?;
                }
                Ok(())
            })
        })
        .await;
        if matches!(removed, Ok(Ok(()))) {
            report.removed.push(id);
        } else {
            report.failed.push(id);
        }
    }
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndm_core::{NewTask, SchedulerOptions};
    use std::collections::BTreeMap;

    #[tokio::test]
    async fn batch_delete_preserves_prepared_transfers_and_reports_failures() {
        let dir = tempfile::tempdir().unwrap();
        let store = Arc::new(Store::open(&dir.path().join("tasks.db")).unwrap());
        let request = NewTask {
            url: "https://example.test/file".into(),
            destination: dir.path().into(),
            suggested_filename: None,
            source_url: None,
            expected_size: None,
            headers: BTreeMap::new(),
        };
        let paused = [TaskId::new_v4(), TaskId::new_v4()];
        for id in paused {
            store.prepare(id, &request).unwrap();
            store.commit(id).unwrap();
            store.pause(id).unwrap();
            let partial = dir.path().join(format!(".ndm-next-{id}"));
            std::fs::create_dir(&partial).unwrap();
            std::fs::write(partial.join("segment-0.part"), b"partial").unwrap();
        }
        let prepared = TaskId::new_v4();
        store.prepare(prepared, &request).unwrap();
        let scheduler = Scheduler::start(
            store.clone(),
            SchedulerOptions {
                auto_resume: false,
                ..Default::default()
            },
        )
        .unwrap();
        let report = delete_many(
            store.clone(),
            scheduler.clone(),
            vec![paused[0], prepared, paused[1]],
            true,
        )
        .await;
        assert_eq!(report.removed, paused);
        assert_eq!(report.failed, [prepared]);
        assert!(store.get(paused[0]).unwrap().is_none());
        assert!(store.get(paused[1]).unwrap().is_none());
        assert_eq!(
            store.get(prepared).unwrap().unwrap().state,
            TaskState::Prepared
        );
        assert!(!dir.path().join(format!(".ndm-next-{}", paused[0])).exists());
        scheduler.shutdown().await;
    }

    #[tokio::test]
    async fn deleting_a_running_task_stops_the_worker_before_removing_parts() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request).await;
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 10485760\r\nConnection: close\r\n\r\n",
                )
                .await
                .unwrap();
            for _ in 0..640 {
                if stream.write_all(&[7; 16384]).await.is_err() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
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
        let request = NewTask {
            url: format!("http://{address}/file"),
            destination: dir.path().into(),
            suggested_filename: None,
            source_url: None,
            expected_size: None,
            headers: BTreeMap::new(),
        };
        store.prepare(id, &request).unwrap();
        store.commit(id).unwrap();
        scheduler.notify();
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if scheduler
                    .progress(id)
                    .is_some_and(|snapshot| snapshot.downloaded > 0)
                {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("download should begin");
        let report = delete_many(store.clone(), scheduler.clone(), vec![id], true).await;
        assert_eq!(report.removed, [id]);
        assert!(report.failed.is_empty());
        assert!(store.get(id).unwrap().is_none());
        assert!(!dir.path().join(format!(".ndm-next-{id}")).exists());
        scheduler.shutdown().await;
        server.abort();
    }

    #[tokio::test]
    async fn completed_download_with_missing_output_still_removes_its_record() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut request = [0; 4096];
            let _ = stream.read(&mut request).await;
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\nConnection: close\r\n\r\ndata")
                .await
                .unwrap();
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
        let output = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(task) = store.get(id).unwrap()
                    && task.state == TaskState::Completed
                {
                    break task.output_path.unwrap();
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        std::fs::remove_file(output).unwrap();
        let report = delete_many(store.clone(), scheduler.clone(), vec![id], true).await;
        assert_eq!(report.removed, [id]);
        assert!(report.failed.is_empty());
        assert!(store.get(id).unwrap().is_none());
        scheduler.shutdown().await;
        server.await.unwrap();
    }
}
