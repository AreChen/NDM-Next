use ndm_core::{Task, TaskOptions};
use reqwest::Url;

fn url_filename(url: &Url) -> Option<String> {
    let segment = url.path_segments()?.rfind(|name| !name.is_empty())?;
    percent_encoding::percent_decode_str(segment)
        .decode_utf8()
        .ok()
        .map(|name| name.into_owned())
}

pub(crate) fn manual_suggestion(url: &Url, options: &TaskOptions) -> Option<String> {
    let is_hls = matches!(options, TaskOptions::Hls(_))
        || (matches!(options, TaskOptions::Auto)
            && url.path().to_ascii_lowercase().ends_with(".m3u8"));
    if is_hls { url_filename(url) } else { None }
}

pub(crate) fn display_name(task: &Task) -> Option<String> {
    task.output_path
        .as_ref()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .or_else(|| task.request.suggested_filename.clone())
        .or_else(|| {
            Url::parse(&task.request.url)
                .ok()
                .and_then(|url| url_filename(&url))
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndm_core::{NewTask, TaskId, TaskOptions, TaskState};
    use std::{collections::BTreeMap, path::PathBuf};

    fn task(url: &str) -> Task {
        Task {
            id: TaskId::new_v4(),
            request: NewTask {
                url: url.into(),
                destination: PathBuf::from("."),
                suggested_filename: None,
                source_url: None,
                expected_size: None,
                headers: BTreeMap::new(),
            },
            options: TaskOptions::Auto,
            state: TaskState::Queued,
            output_path: None,
            error: None,
            size_verified: None,
        }
    }

    #[test]
    fn manual_http_uses_the_server_filename_and_displays_the_final_name() {
        let url = reqwest::Url::parse("https://example.test/download?id=42").unwrap();
        assert_eq!(manual_suggestion(&url, &TaskOptions::Auto), None);
        let mut item = task(url.as_str());
        assert_eq!(display_name(&item).as_deref(), Some("download"));
        item.output_path = Some(PathBuf::from("setup.exe"));
        assert_eq!(display_name(&item).as_deref(), Some("setup.exe"));
    }

    #[test]
    fn display_and_hls_names_decode_non_ascii_url_segments() {
        let url = reqwest::Url::parse("https://example.test/%E4%B8%8B%E8%BD%BD.m3u8").unwrap();
        assert_eq!(
            manual_suggestion(&url, &TaskOptions::Auto).as_deref(),
            Some("下载.m3u8")
        );
        assert_eq!(
            display_name(&task(url.as_str())).as_deref(),
            Some("下载.m3u8")
        );
    }
}
