//! Search metadata is built when the task list reloads, never while rendering a row.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use caseless::Caseless;
use ndm_core::{Task, TaskId, TaskState};
use unicode_normalization::UnicodeNormalization;

// Reject overlong input instead of silently discarding AND terms. The limits
// also bound normalization, n-gram construction and posting-list intersections.
const MAX_QUERY_BYTES: usize = 4_096;
const MAX_QUERY_TOKENS: usize = 64;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum Category {
    #[default]
    All,
    Archives,
    Documents,
    Video,
    Audio,
    Images,
    Applications,
    Other,
}

impl Category {
    pub(crate) const ALL: [Self; 8] = [
        Self::All,
        Self::Archives,
        Self::Documents,
        Self::Video,
        Self::Audio,
        Self::Images,
        Self::Applications,
        Self::Other,
    ];

    pub(crate) fn label(self, chinese: bool) -> &'static str {
        match (self, chinese) {
            (Self::All, true) => "全部类型",
            (Self::All, false) => "All types",
            (Self::Archives, true) => "压缩包",
            (Self::Archives, false) => "Archives",
            (Self::Documents, true) => "文档",
            (Self::Documents, false) => "Documents",
            (Self::Video, true) => "视频",
            (Self::Video, false) => "Video",
            (Self::Audio, true) => "音频",
            (Self::Audio, false) => "Audio",
            (Self::Images, true) => "图片",
            (Self::Images, false) => "Images",
            (Self::Applications, true) => "应用程序",
            (Self::Applications, false) => "Applications",
            (Self::Other, true) => "其他",
            (Self::Other, false) => "Other",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum SortOrder {
    #[default]
    Recent,
    Name,
    Size,
}

impl SortOrder {
    pub(crate) const ALL: [Self; 3] = [Self::Recent, Self::Name, Self::Size];

    pub(crate) fn label(self, chinese: bool) -> &'static str {
        match (self, chinese) {
            (Self::Recent, true) => "最近添加",
            (Self::Recent, false) => "Recent",
            (Self::Name, true) => "名称",
            (Self::Name, false) => "Name",
            (Self::Size, true) => "大小",
            (Self::Size, false) => "Size",
        }
    }
}

#[derive(Clone, Copy, Eq, Hash, PartialEq)]
enum Gram {
    One(char),
    Two(char, char),
    Three(char, char, char),
}

struct IndexedTask {
    // The first field is the displayed filename, also used as the sort key.
    // Keeping fields separate prevents matches across filename/URL boundaries.
    fields: Vec<String>,
    metadata: SearchMetadata,
    size: Option<u64>,
}

struct CompletedOutputSize {
    path: PathBuf,
    size: u64,
}

struct SearchMetadata {
    id: TaskId,
    url: String,
    source_url: Option<String>,
    suggested_filename: Option<String>,
    engine_title: Option<String>,
    output_path: Option<PathBuf>,
    size: Option<u64>,
}

impl SearchMetadata {
    fn from_task(task: &Task) -> Self {
        Self {
            id: task.id,
            url: task.request.url.clone(),
            source_url: task.request.source_url.clone(),
            suggested_filename: task.request.suggested_filename.clone(),
            engine_title: task.engine_title.clone(),
            output_path: task.output_path.clone(),
            size: task.request.expected_size,
        }
    }

    fn matches(&self, task: &Task) -> bool {
        self.id == task.id
            && self.url == task.request.url
            && self.source_url == task.request.source_url
            && self.suggested_filename == task.request.suggested_filename
            && self.engine_title == task.engine_title
            && self.output_path == task.output_path
            && self.size == task.request.expected_size
    }
}

#[derive(Default)]
pub(crate) struct TaskIndex {
    documents: Vec<IndexedTask>,
    postings: HashMap<Gram, Vec<usize>>,
    categories: [Vec<usize>; 8],
    positions: HashMap<TaskId, usize>,
    completed_sizes: HashMap<TaskId, CompletedOutputSize>,
}

impl TaskIndex {
    /// A pure cached lookup for task rows. Completed output length takes
    /// precedence over the caller's expected size; this never touches disk.
    pub(crate) fn known_size(&self, id: TaskId) -> Option<u64> {
        self.positions
            .get(&id)
            .and_then(|&position| self.documents.get(position))
            .and_then(|document| document.size)
    }

    /// Update at reload, reusing normalized text for status/progress-only changes.
    /// A completion or a previously missing output may change the cached size.
    /// The return value can invalidate cached filter/sort results.
    pub(crate) fn update(&mut self, tasks: &[Task]) -> bool {
        if self.documents.len() == tasks.len()
            && self
                .documents
                .iter()
                .zip(tasks)
                .all(|(document, task)| document.metadata.matches(task))
        {
            self.refresh_completed_sizes(tasks);
            let mut sizes_changed = false;
            for (document, task) in self.documents.iter_mut().zip(tasks) {
                let size = self
                    .completed_sizes
                    .get(&task.id)
                    .map(|output| output.size)
                    .or(task.request.expected_size);
                sizes_changed |= document.size != size;
                document.size = size;
            }
            return sizes_changed;
        }
        self.rebuild(tasks);
        true
    }

    /// Rebuild after reloading the task slice. Returned positions remain valid
    /// until that slice changes. `Recent` assumes Store::list's insertion order.
    pub(crate) fn rebuild(&mut self, tasks: &[Task]) {
        self.positions.clear();
        self.positions.extend(
            tasks
                .iter()
                .enumerate()
                .map(|(position, task)| (task.id, position)),
        );
        self.refresh_completed_sizes(tasks);
        self.documents.clear();
        self.postings.clear();
        for positions in &mut self.categories {
            positions.clear();
        }

        let mut grams = HashSet::new();
        for (position, task) in tasks.iter().enumerate() {
            let name = crate::filenames::display_name(task).unwrap_or_default();
            let name = normalize(&name);
            let category = classify_name(&name);
            let mut fields = vec![name];
            add_url_fields(&mut fields, &task.request.url);
            if let Some(source) = &task.request.source_url {
                add_url_fields(&mut fields, source);
            }

            grams.clear();
            for field in &fields {
                let chars: Vec<_> = field.chars().collect();
                grams.extend(chars.iter().copied().map(Gram::One));
                grams.extend(chars.windows(2).map(|c| Gram::Two(c[0], c[1])));
                grams.extend(chars.windows(3).map(|c| Gram::Three(c[0], c[1], c[2])));
            }
            // Add each position once, in ascending order, for fast intersection.
            for gram in &grams {
                self.postings.entry(*gram).or_default().push(position);
            }
            self.categories[category as usize].push(position);
            self.documents.push(IndexedTask {
                fields,
                metadata: SearchMetadata::from_task(task),
                size: self
                    .completed_sizes
                    .get(&task.id)
                    .map(|output| output.size)
                    .or(task.request.expected_size),
            });
        }
    }

    // Run only at task-list reload, never from filter(), known_size() or paint.
    // Keep successful observations across full search rebuilds. Failed stats
    // remain uncached so a later reload can discover a newly available output.
    fn refresh_completed_sizes(&mut self, tasks: &[Task]) {
        let positions = &self.positions;
        self.completed_sizes.retain(|id, output| {
            positions
                .get(id)
                .and_then(|&position| tasks.get(position))
                .is_some_and(|task| {
                    task.state == TaskState::Completed
                        && task.output_path.as_ref() == Some(&output.path)
                })
        });
        for task in tasks {
            if task.state != TaskState::Completed || self.completed_sizes.contains_key(&task.id) {
                continue;
            }
            if let Some(path) = &task.output_path
                && let Ok(metadata) = std::fs::metadata(path)
                && metadata.is_file()
            {
                self.completed_sizes.insert(
                    task.id,
                    CompletedOutputSize {
                        path: path.clone(),
                        size: metadata.len(),
                    },
                );
            }
        }
    }

    /// Whitespace-separated terms use AND semantics; each term may match any
    /// indexed field. Normalization is compatibility + full Unicode case folding,
    /// followed by canonical recomposition; accents are preserved, not removed.
    /// This is literal matching, without translation, pinyin or locale collation.
    pub(crate) fn filter(&self, query: &str, category: Category, sort: SortOrder) -> Vec<usize> {
        self.filter_with_totals(query, category, sort, &HashMap::new())
    }

    /// Live progress totals supply a size only when the cached output/request
    /// has none. The snapshot map is read directly, without IO or reindexing.
    pub(crate) fn filter_with_totals(
        &self,
        query: &str,
        category: Category,
        sort: SortOrder,
        discovered: &HashMap<TaskId, u64>,
    ) -> Vec<usize> {
        if query.len() > MAX_QUERY_BYTES {
            return Vec::new();
        }
        let normalized_query = normalize(query);
        let tokens: Vec<_> = normalized_query
            .split_whitespace()
            .take(MAX_QUERY_TOKENS + 1)
            .collect();
        if tokens.len() > MAX_QUERY_TOKENS {
            return Vec::new();
        }

        let mut grams = HashSet::new();
        for token in &tokens {
            let chars: Vec<_> = token.chars().collect();
            match chars.as_slice() {
                [a] => {
                    grams.insert(Gram::One(*a));
                }
                [a, b] => {
                    grams.insert(Gram::Two(*a, *b));
                }
                _ => grams.extend(chars.windows(3).map(|c| Gram::Three(c[0], c[1], c[2]))),
            }
        }
        let mut lists = Vec::with_capacity(grams.len() + 1);
        if category != Category::All {
            lists.push(self.categories[category as usize].as_slice());
        }
        for gram in grams {
            let Some(positions) = self.postings.get(&gram) else {
                return Vec::new();
            };
            lists.push(positions.as_slice());
        }
        lists.sort_unstable_by_key(|positions| positions.len());
        let mut candidates = match lists.first() {
            Some(first) => first.to_vec(),
            None => (0..self.documents.len()).collect(),
        };
        for positions in lists.iter().skip(1) {
            intersect(&mut candidates, positions);
            if candidates.is_empty() {
                return candidates;
            }
        }
        // Grams can occur in a different order or in different fields. Verify
        // only the surviving candidates against their already normalized text.
        candidates.retain(|&position| {
            tokens.iter().all(|token| {
                self.documents[position]
                    .fields
                    .iter()
                    .any(|field| field.contains(token))
            })
        });
        match sort {
            SortOrder::Recent => candidates.reverse(),
            SortOrder::Name => candidates.sort_unstable_by(|&a, &b| {
                self.documents[a].fields[0]
                    .cmp(&self.documents[b].fields[0])
                    .then_with(|| b.cmp(&a))
            }),
            // Known sizes descend, zero precedes unknown, newest breaks ties.
            SortOrder::Size => {
                let size = |position: usize| {
                    let document = &self.documents[position];
                    document
                        .size
                        .or_else(|| discovered.get(&document.metadata.id).copied())
                };
                candidates.sort_unstable_by(|&a, &b| size(b).cmp(&size(a)).then_with(|| b.cmp(&a)));
            }
        }
        candidates
    }
}

pub(crate) fn normalize(value: &str) -> String {
    value.nfkc().default_case_fold().nfkc().collect()
}

fn add_url_fields(fields: &mut Vec<String>, url: &str) {
    let raw = normalize(url);
    let decoded = percent_encoding::percent_decode_str(url).decode_utf8_lossy();
    let decoded = normalize(&decoded);
    for field in [raw, decoded] {
        if !field.is_empty() && !fields.contains(&field) {
            fields.push(field);
        }
    }
}

fn intersect(candidates: &mut Vec<usize>, positions: &[usize]) {
    if candidates.len().saturating_mul(8) < positions.len() {
        candidates.retain(|position| positions.binary_search(position).is_ok());
    } else {
        let mut other = 0;
        candidates.retain(|position| {
            while other < positions.len() && positions[other] < *position {
                other += 1;
            }
            positions.get(other) == Some(position)
        });
    }
}

fn classify_name(normalized_name: &str) -> Category {
    let extension = normalized_name
        .rsplit_once('.')
        .map(|(_, ext)| ext)
        .unwrap_or("");
    match extension {
        "zip" | "7z" | "rar" | "tar" | "gz" | "bz2" | "xz" | "zst" | "tgz" | "tbz" | "tbz2"
        | "txz" | "lz" | "lzma" | "cab" | "iso" => Category::Archives,
        "pdf" | "txt" | "rtf" | "md" | "csv" | "tsv" | "doc" | "docx" | "xls" | "xlsx" | "ppt"
        | "pptx" | "odt" | "ods" | "odp" | "epub" | "mobi" | "azw" | "azw3" | "pages"
        | "numbers" | "key" | "tex" | "html" | "htm" | "xml" | "json" | "log" | "yaml" | "yml" => {
            Category::Documents
        }
        "mp4" | "mkv" | "webm" | "mov" | "avi" | "m4v" | "wmv" | "flv" | "mpeg" | "mpg" | "ts"
        | "m2ts" | "mts" | "ogv" | "vob" | "m3u8" | "mpd" => Category::Video,
        "mp3" | "m4a" | "aac" | "flac" | "wav" | "ogg" | "oga" | "opus" | "wma" | "aiff"
        | "aif" | "alac" | "ape" => Category::Audio,
        "jpg" | "jpeg" | "png" | "gif" | "webp" | "avif" | "bmp" | "svg" | "ico" | "tif"
        | "tiff" | "heic" | "heif" | "jxl" | "psd" => Category::Images,
        "exe" | "msi" | "msix" | "msixbundle" | "appx" | "appxbundle" | "appinstaller" | "apk"
        | "aab" | "dmg" | "pkg" | "app" | "appimage" | "deb" | "rpm" | "jar" => {
            Category::Applications
        }
        _ => Category::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ndm_core::{NewTask, TaskId, TaskOptions, TaskState};
    use std::{collections::BTreeMap, path::PathBuf};
    use tempfile::TempDir;

    fn task(name: Option<&str>, url: &str) -> Task {
        Task {
            cache_directory: None,
            policy: Default::default(),
            id: TaskId::new_v4(),
            request: NewTask {
                url: url.into(),
                destination: PathBuf::from("."),
                suggested_filename: name.map(str::to_owned),
                source_url: None,
                expected_size: None,
                headers: BTreeMap::new(),
            },
            options: TaskOptions::Auto,
            state: TaskState::Queued,
            engine_title: None,
            output_path: None,
            error: None,
            size_verified: None,
            progress: None,
        }
    }

    fn build_index(tasks: &[Task]) -> TaskIndex {
        let mut index = TaskIndex::default();
        index.rebuild(tasks);
        index
    }

    #[test]
    fn normalizes_chinese_full_width_case_and_canonically_equivalent_accents() {
        let tasks = [
            task(
                Some("ＡＢＣ Re\u{301}sume\u{301} 中文.pdf"),
                "https://files.test/1",
            ),
            task(Some("其他.txt"), "https://files.test/2"),
        ];
        let index = build_index(&tasks);
        assert_eq!(
            index.filter("abc RÉSUMÉ 中文", Category::All, SortOrder::Recent),
            [0]
        );
        assert_eq!(index.filter("文", Category::All, SortOrder::Recent), [0]);
    }

    #[test]
    fn folds_expanding_and_final_sigma_case_variants() {
        let index = build_index(&[task(Some("Straße Σίσυφος.txt"), "https://files.test/a")]);
        assert_eq!(
            index.filter("STRASSE ΣΊΣΥΦΟΣ", Category::All, SortOrder::Recent),
            [0]
        );
    }

    #[test]
    fn searches_decoded_url_names_raw_urls_and_source_with_and_tokens() {
        let mut item = task(
            None,
            "https://cdn.test/%E4%B8%AD%E6%96%87%20Caf%C3%A9.PDF?id=42",
        );
        item.request.source_url = Some("https://Pages.Example.org/watch?kind=Lecture".into());
        let index = build_index(&[item]);
        assert_eq!(
            index.filter(
                "中文 café pages.example LECTURE",
                Category::All,
                SortOrder::Recent
            ),
            [0]
        );
        assert_eq!(
            index.filter("%E4%B8%AD id=42", Category::All, SortOrder::Recent),
            [0]
        );
        assert!(
            index
                .filter("中文 missing", Category::All, SortOrder::Recent)
                .is_empty()
        );
    }

    #[test]
    fn requires_exact_substrings_after_intersecting_grams() {
        let index = build_index(&[
            task(Some("abcXbcd.txt"), "https://a.test/1"),
            task(Some("abcd.txt"), "https://a.test/2"),
        ]);
        assert_eq!(index.filter("abcd", Category::All, SortOrder::Recent), [1]);
        assert_eq!(
            index.filter("  ab\tbc\n", Category::All, SortOrder::Recent),
            [1, 0]
        );
        assert!(
            index
                .filter("txthttps", Category::All, SortOrder::Recent)
                .is_empty()
        );
    }

    #[test]
    fn classifies_real_names_and_ignores_url_query_extensions() {
        let cases = [
            ("backup.TAR.GZ", Category::Archives),
            ("notes.ＰＤＦ", Category::Documents),
            ("clip.MKV", Category::Video),
            ("track.flac", Category::Audio),
            ("image.webp", Category::Images),
            ("setup.MSIX", Category::Applications),
            ("README", Category::Other),
        ];
        let tasks: Vec<_> = cases
            .iter()
            .map(|(name, _)| task(Some(name), "https://a.test/get?file=movie.mp4"))
            .collect();
        let index = build_index(&tasks);
        for (position, (_, category)) in cases.iter().enumerate() {
            assert_eq!(index.filter("", *category, SortOrder::Recent), [position]);
        }
        let mut saved = task(Some("report.pdf"), "https://a.test/old.mp4");
        saved.output_path = Some(PathBuf::from("saved.zip"));
        let index = build_index(&[
            saved,
            task(None, "https://a.test/%E4%B8%AD%E6%96%87.PDF?q=.mp4"),
            task(None, "https://a.test/get?file=movie.mp4"),
        ]);
        assert_eq!(index.filter("", Category::Archives, SortOrder::Recent), [0]);
        assert_eq!(
            index.filter("中文", Category::Documents, SortOrder::Recent),
            [1]
        );
        assert_eq!(index.filter("", Category::Other, SortOrder::Recent), [2]);
    }

    #[test]
    fn sorts_recent_name_and_known_sizes_without_touching_the_filesystem() {
        let mut tasks = vec![
            task(Some("Ｂeta.zip"), "https://a.test/1"),
            task(Some("alpha.pdf"), "https://a.test/2"),
            task(Some("ALPHA.txt"), "https://a.test/3"),
            task(Some("empty.bin"), "https://a.test/4"),
        ];
        tasks[0].request.expected_size = Some(10);
        tasks[1].request.expected_size = Some(100);
        tasks[3].request.expected_size = Some(0);
        let index = build_index(&tasks);
        assert_eq!(
            index.filter("", Category::All, SortOrder::Recent),
            [3, 2, 1, 0]
        );
        assert_eq!(
            index.filter("", Category::All, SortOrder::Name),
            [1, 2, 0, 3]
        );
        assert_eq!(
            index.filter("", Category::All, SortOrder::Size),
            [1, 0, 3, 2]
        );
    }

    #[test]
    fn completed_sizes_use_actual_output_lengths_before_request_estimates() {
        let directory = TempDir::new().unwrap();
        let small = directory.path().join("small.bin");
        let large = directory.path().join("large.bin");
        std::fs::write(&small, b"123").unwrap();
        std::fs::write(&large, b"12345678").unwrap();
        let mut tasks = vec![
            task(Some("small.bin"), "https://a.test/1"),
            task(Some("large.bin"), "https://a.test/2"),
            task(Some("queued.bin"), "https://a.test/3"),
        ];
        tasks[0].state = TaskState::Completed;
        tasks[0].output_path = Some(small);
        tasks[0].request.expected_size = Some(999);
        tasks[1].state = TaskState::Completed;
        tasks[1].output_path = Some(large);
        tasks[2].request.expected_size = Some(5);
        let index = build_index(&tasks);
        assert_eq!(index.known_size(tasks[0].id), Some(3));
        assert_eq!(index.known_size(tasks[1].id), Some(8));
        assert_eq!(index.known_size(tasks[2].id), Some(5));
        assert_eq!(index.known_size(TaskId::new_v4()), None);
        assert_eq!(index.filter("", Category::All, SortOrder::Size), [1, 2, 0]);
        assert_eq!(tasks[0].request.expected_size, Some(999));
        assert_eq!(tasks[1].request.expected_size, None);
    }

    #[test]
    fn completing_an_unknown_size_task_discovers_its_published_size() {
        let directory = TempDir::new().unwrap();
        let output = directory.path().join("finished.zip");
        std::fs::write(&output, b"1234567").unwrap();
        let mut tasks = vec![task(Some("finished.zip"), "https://a.test/1")];
        tasks[0].output_path = Some(output);
        let mut index = build_index(&tasks);
        assert_eq!(index.known_size(tasks[0].id), None);
        tasks[0].state = TaskState::Completed;
        assert!(index.update(&tasks));
        assert_eq!(index.known_size(tasks[0].id), Some(7));
        assert_eq!(
            index.filter("finished", Category::Archives, SortOrder::Recent),
            [0]
        );
        assert!(!index.update(&tasks));
    }

    #[test]
    fn discovered_totals_sort_active_tasks_until_completed_output_takes_precedence() {
        let directory = TempDir::new().unwrap();
        let output = directory.path().join("Zulu.bin");
        std::fs::write(&output, b"123").unwrap();
        let mut tasks = vec![
            task(Some("Zulu.bin"), "https://a.test/manual/1"),
            task(Some("Alpha.bin"), "https://a.test/manual/2"),
        ];
        for task in &mut tasks {
            task.state = TaskState::Running;
        }
        let mut index = build_index(&tasks);
        let discovered = HashMap::from([(tasks[0].id, 50), (tasks[1].id, 5)]);
        assert_eq!(
            index.filter_with_totals("manual", Category::Other, SortOrder::Size, &discovered),
            [0, 1]
        );
        assert_eq!(index.filter("", Category::All, SortOrder::Size), [1, 0]);
        for sort in [SortOrder::Name, SortOrder::Recent] {
            assert_eq!(
                index.filter_with_totals("", Category::All, sort, &discovered),
                [1, 0]
            );
        }

        tasks[0].state = TaskState::Completed;
        tasks[0].output_path = Some(output);
        assert!(index.update(&tasks));
        assert_eq!(index.known_size(tasks[0].id), Some(3));
        assert_eq!(
            index.filter_with_totals("", Category::All, SortOrder::Size, &discovered),
            [1, 0]
        );
        assert_eq!(tasks[0].request.expected_size, None);
        assert_eq!(tasks[1].request.expected_size, None);
    }

    #[test]
    fn missing_completed_output_is_retried_on_a_later_update() {
        let directory = TempDir::new().unwrap();
        let output = directory.path().join("appears-later.bin");
        let mut tasks = vec![task(None, "https://a.test/1")];
        tasks[0].state = TaskState::Completed;
        tasks[0].output_path = Some(output.clone());
        tasks[0].request.expected_size = Some(100);
        let mut index = build_index(&tasks);
        assert_eq!(index.known_size(tasks[0].id), Some(100));
        assert!(!index.update(&tasks));
        std::fs::write(&output, b"1234").unwrap();
        assert!(index.update(&tasks));
        assert_eq!(index.known_size(tasks[0].id), Some(4));
    }

    #[test]
    fn successful_size_cache_survives_reloads_and_invalidates_for_a_new_output_path() {
        let directory = TempDir::new().unwrap();
        let first = directory.path().join("first.bin");
        let second = directory.path().join("second.bin");
        std::fs::write(&first, b"123").unwrap();
        std::fs::write(&second, b"123456").unwrap();
        let mut tasks = vec![task(None, "https://a.test/1")];
        tasks[0].state = TaskState::Completed;
        tasks[0].output_path = Some(first.clone());
        let mut index = build_index(&tasks);
        assert_eq!(index.known_size(tasks[0].id), Some(3));

        // A successful stat is cached for this task/path, including full search
        // rebuilds. Neither the row accessor nor state reloads stat it again.
        std::fs::write(&first, b"changed after the initial stat").unwrap();
        assert!(!index.update(&tasks));
        index.rebuild(&tasks);
        assert_eq!(index.known_size(tasks[0].id), Some(3));
        tasks[0].output_path = Some(second);
        assert!(index.update(&tasks));
        assert_eq!(index.known_size(tasks[0].id), Some(6));

        let id = tasks[0].id;
        assert!(index.update(&[]));
        assert_eq!(index.known_size(id), None);
        std::fs::write(tasks[0].output_path.as_ref().unwrap(), b"new").unwrap();
        assert!(index.update(&tasks));
        assert_eq!(index.known_size(id), Some(3));
    }

    #[test]
    fn rebuild_replaces_removed_tasks_and_renamed_metadata() {
        let mut tasks = vec![
            task(Some("old.zip"), "https://a.test/1"),
            task(Some("removed.pdf"), "https://a.test/2"),
        ];
        let mut index = build_index(&tasks);
        tasks.truncate(1);
        tasks[0].request.suggested_filename = Some("new.txt".into());
        index.rebuild(&tasks);
        assert!(
            index
                .filter("old", Category::All, SortOrder::Recent)
                .is_empty()
        );
        assert!(
            index
                .filter("removed", Category::All, SortOrder::Recent)
                .is_empty()
        );
        assert_eq!(
            index.filter("new", Category::Documents, SortOrder::Recent),
            [0]
        );
    }

    #[test]
    fn update_reports_changes_to_search_metadata_but_skips_state_only_reloads() {
        let mut tasks = vec![
            task(Some("first.txt"), "https://a.test/first"),
            task(Some("second.txt"), "https://a.test/second"),
        ];
        let mut index = TaskIndex::default();
        assert!(index.update(&tasks));
        tasks[0].state = TaskState::Completed;
        tasks[0].error = Some("different status".into());
        tasks[0].size_verified = Some(true);
        assert!(!index.update(&tasks));

        tasks[0].request.expected_size = Some(10);
        assert!(index.update(&tasks));
        assert_eq!(index.filter("", Category::All, SortOrder::Size), [0, 1]);
        tasks[0].request.suggested_filename = Some("renamed.txt".into());
        assert!(index.update(&tasks));
        assert_eq!(
            index.filter("renamed", Category::All, SortOrder::Recent),
            [0]
        );
        tasks[0].request.url = "https://b.test/new-address".into();
        assert!(index.update(&tasks));
        assert_eq!(
            index.filter("new-address", Category::All, SortOrder::Recent),
            [0]
        );
        tasks[0].request.source_url = Some("https://source.test/original".into());
        assert!(index.update(&tasks));
        assert_eq!(
            index.filter("original", Category::All, SortOrder::Recent),
            [0]
        );
        tasks[0].output_path = Some(PathBuf::from("final.zip"));
        assert!(index.update(&tasks));
        assert_eq!(
            index.filter("final", Category::Archives, SortOrder::Recent),
            [0]
        );
        tasks.swap(0, 1);
        assert!(index.update(&tasks));
        assert_eq!(
            index.filter("final", Category::Archives, SortOrder::Recent),
            [1]
        );
        tasks.truncate(1);
        assert!(index.update(&tasks));
        assert!(
            index
                .filter("final", Category::All, SortOrder::Recent)
                .is_empty()
        );
        assert!(!index.update(&tasks));
    }

    #[test]
    fn bounds_excessive_query_input_without_silently_dropping_and_terms() {
        let index = build_index(&[task(Some("match.txt"), "https://a.test/1")]);
        assert!(
            index
                .filter(&"match ".repeat(10_000), Category::All, SortOrder::Recent)
                .is_empty()
        );
        assert!(
            index
                .filter(&"match ".repeat(65), Category::All, SortOrder::Recent)
                .is_empty()
        );
        assert_eq!(
            index.filter("\u{3000}\t", Category::All, SortOrder::Recent),
            [0]
        );
    }

    #[test]
    fn finds_short_and_long_tokens_in_a_large_mixed_corpus() {
        let mut tasks: Vec<_> = (0..2_000)
            .map(|n| {
                task(
                    Some(&format!("文件{n:04}.txt")),
                    &format!("https://cdn.test/{n}"),
                )
            })
            .collect();
        tasks[1_337].request.suggested_filename = Some("特殊 CAFÉ Straße.zip".into());
        let index = build_index(&tasks);
        assert_eq!(
            index.filter("特 café STRASSE cdn", Category::Archives, SortOrder::Recent),
            [1_337]
        );
        assert_eq!(
            index.filter("文件1999", Category::Documents, SortOrder::Recent),
            [1_999]
        );
    }
}
