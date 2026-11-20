use std::fs;

use crate::{ProgressSnapshot, SegmentSnapshot, Task, TaskState, secrets};

/// Read progress for a task created before progress snapshots were stored.
/// This is presentation data only; protocol recovery still validates its own files.
pub fn recover_task_progress(task: &Task) -> Option<ProgressSnapshot> {
    if let Some(progress) = &task.progress {
        return Some(progress.clone());
    }
    if task.state == TaskState::Completed {
        let metadata = fs::metadata(task.output_path.as_ref()?).ok()?;
        return metadata.is_file().then(|| ProgressSnapshot {
            downloaded: metadata.len(),
            total: Some(metadata.len()),
            ..Default::default()
        });
    }
    if !matches!(task.state, TaskState::Paused | TaskState::Failed) {
        return None;
    }
    recover_partial_progress(task)
}

pub(crate) fn recover_partial_progress(task: &Task) -> Option<ProgressSnapshot> {
    let directory = task.work_directory();
    let mut segments = Vec::new();
    for entry in fs::read_dir(&directory).ok()?.flatten() {
        let name = entry.file_name();
        let name = name.to_str().unwrap_or_default();
        let index = name
            .strip_prefix("segment-")
            .and_then(|name| name.strip_suffix(".part"))
            .and_then(|index| index.parse::<usize>().ok());
        if let Some(index) = index
            && let Ok(metadata) = entry.metadata()
            && metadata.is_file()
        {
            segments.push((
                index,
                SegmentSnapshot {
                    downloaded: metadata.len(),
                    total: task
                        .progress
                        .as_ref()
                        .and_then(|progress| progress.segments.get(index))
                        .and_then(|segment| segment.total),
                    active: false,
                },
            ));
        }
    }
    segments.sort_by_key(|(index, _)| *index);
    let segments: Vec<_> = segments.into_iter().map(|(_, segment)| segment).collect();
    let downloaded = if segments.is_empty() {
        ["assembled.part", "ftp.part", "sftp.part"]
            .into_iter()
            .filter_map(|name| fs::metadata(directory.join(name)).ok())
            .filter(|metadata| metadata.is_file())
            .map(|metadata| metadata.len())
            .max()?
    } else {
        segments
            .iter()
            .fold(0u64, |sum, segment| sum.saturating_add(segment.downloaded))
    };
    let total = fs::read(directory.join("resume.bin"))
        .ok()
        .and_then(|data| secrets::unprotect(&data).ok())
        .and_then(|data| serde_json::from_slice::<serde_json::Value>(&data).ok())
        .and_then(|value| value.get("total").and_then(serde_json::Value::as_u64))
        .or(task.request.expected_size)
        .or_else(|| task.progress.as_ref().and_then(|progress| progress.total));
    Some(ProgressSnapshot {
        downloaded,
        total,
        segments,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{NewTask, TaskId, TaskOptions};

    #[test]
    fn legacy_segment_progress_does_not_count_assembly_twice_or_invent_timing() {
        let directory = tempfile::tempdir().unwrap();
        let task = Task {
            cache_directory: None,
            policy: Default::default(),
            id: TaskId::new_v4(),
            request: NewTask {
                url: "https://example.test/file".into(),
                destination: directory.path().into(),
                suggested_filename: None,
                source_url: None,
                expected_size: None,
                headers: Default::default(),
            },
            options: TaskOptions::Auto,
            state: TaskState::Paused,
            output_path: None,
            error: None,
            size_verified: None,
            progress: None,
        };
        let parts = directory.path().join(format!(".ndm-next-{}", task.id));
        fs::create_dir(&parts).unwrap();
        fs::write(parts.join("segment-0.part"), [0; 20]).unwrap();
        fs::write(parts.join("segment-1.part"), [0; 30]).unwrap();
        fs::write(parts.join("assembled.part"), [0; 10]).unwrap();
        fs::write(parts.join("segment-invalid.part"), [0; 99]).unwrap();
        fs::write(
            parts.join("resume.bin"),
            secrets::protect(br#"{"total":100}"#).unwrap(),
        )
        .unwrap();
        let progress = recover_task_progress(&task).unwrap();
        assert_eq!(progress.downloaded, 50);
        assert_eq!(progress.total, Some(100));
        assert_eq!(progress.elapsed_ms, 0);
        assert_eq!(progress.average_bytes_per_second(), None);
    }
}
