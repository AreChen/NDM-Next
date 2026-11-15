use std::{
    fs, io,
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Clone, Copy, Debug, Default)]
pub struct CacheReport {
    pub bytes: u64,
    pub reclaimable_bytes: u64,
    pub protected_bytes: u64,
    pub removed_bytes: u64,
    pub removed_files: u64,
    pub skipped_directories: u64,
}

pub(crate) fn is_reparse(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    }
    #[cfg(not(windows))]
    {
        metadata.is_symlink()
    }
}

pub(crate) fn validate_directory(directory: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(directory)?;
    if !metadata.is_dir() || is_reparse(&metadata) {
        return Err(io::Error::other(
            "cache directory is not a regular directory",
        ));
    }
    Ok(())
}

pub(crate) struct OwnedFiles {
    pub bytes: u64,
    pub modified: SystemTime,
    files: Vec<PathBuf>,
    directories: Vec<PathBuf>,
}

fn known_file(name: &str, media: bool) -> bool {
    if media {
        return matches!(name, "output.mp4" | "output.mkv")
            || name
                .strip_prefix("track-")
                .and_then(|name| name.strip_suffix(".bin"))
                .and_then(|index| index.parse::<usize>().ok())
                .is_some_and(|index| index < 64 && name == format!("track-{index}.bin"));
    }
    matches!(
        name,
        "resume.bin"
            | "assembled.part"
            | "ftp.part"
            | "sftp.part"
            | "protocol-resume.bin"
            | "protocol-resume.pending"
            | "hls-output.mp4"
            | "hls-output.mp4.hls-partial"
            | "hls-checkpoint.sqlite"
            | "hls-checkpoint.sqlite-journal"
            | "hls-checkpoint.sqlite-wal"
            | "hls-checkpoint.sqlite-shm"
    ) || name
        .strip_prefix("segment-")
        .and_then(|name| name.strip_suffix(".part"))
        .and_then(|index| index.parse::<usize>().ok())
        .is_some_and(|index| index < 16 && name == format!("segment-{index}.part"))
        || crate::dash::owns_partial_name(name)
}

fn collect(directory: &Path, media: bool, result: &mut OwnedFiles) -> io::Result<()> {
    validate_directory(directory)?;
    result.modified = result
        .modified
        .max(fs::symlink_metadata(directory)?.modified()?);
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let metadata = fs::symlink_metadata(&path)?;
        if is_reparse(&metadata) {
            return Err(io::Error::other("linked cache entry"));
        }
        if metadata.is_file() && known_file(&name, media) {
            result.bytes = result.bytes.saturating_add(metadata.len());
            result.modified = result.modified.max(metadata.modified()?);
            result.files.push(path);
        } else if !media
            && metadata.is_dir()
            && name
                .strip_prefix("media-")
                .and_then(|id| crate::TaskId::parse_str(id).ok())
                .is_some_and(|id| name == format!("media-{id}"))
        {
            collect(&path, true, result)?;
        } else {
            return Err(io::Error::other("unexpected cache entry"));
        }
        if result.files.len() > 70_000 {
            return Err(io::Error::other("cache inventory limit exceeded"));
        }
    }
    result.directories.push(directory.to_owned());
    Ok(())
}

pub(crate) fn inspect(root: &Path, id: crate::TaskId) -> io::Result<Option<OwnedFiles>> {
    if !root.is_absolute() {
        return Err(io::Error::other("relative cache root"));
    }
    let directory = root.join(format!(".ndm-next-{id}"));
    match fs::symlink_metadata(&directory) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
        Ok(_) => {}
    }
    validate_directory(root)?;
    let mut result = OwnedFiles {
        bytes: 0,
        modified: SystemTime::UNIX_EPOCH,
        files: Vec::new(),
        directories: Vec::new(),
    };
    collect(&directory, false, &mut result)?;
    Ok(Some(result))
}

impl OwnedFiles {
    /// The complete inventory is validated before removing any entries.
    pub fn remove(self) -> io::Result<u64> {
        let count = self.files.len() as u64;
        for file in self.files {
            fs::remove_file(file)?;
        }
        for directory in self.directories {
            fs::remove_dir(directory)?;
        }
        Ok(count)
    }
}

pub(crate) fn remove_partial(root: &Path, id: crate::TaskId) -> crate::Result<()> {
    if let Some(files) = inspect(root, id)? {
        files.remove()?;
    }
    Ok(())
}
