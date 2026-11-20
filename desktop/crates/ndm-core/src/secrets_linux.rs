use std::{
    fs::{self, DirBuilder, File, OpenOptions},
    io::{self, Read, Write},
    os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt},
    path::Path,
    sync::Mutex,
};

pub(super) fn key(create: bool) -> io::Result<[u8; 32]> {
    static KEY: Mutex<Option<[u8; 32]>> = Mutex::new(None);
    let mut cached = KEY
        .lock()
        .map_err(|_| io::Error::other("credential lock poisoned"))?;
    if let Some(key) = *cached {
        return Ok(key);
    }
    let key = load_key(&crate::application_data_directory(), create)?;
    *cached = Some(key);
    Ok(key)
}

fn load_key(directory: &Path, create: bool) -> io::Result<[u8; 32]> {
    if create {
        DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(directory)?;
    }
    let metadata = fs::symlink_metadata(directory)?;
    // SAFETY: geteuid reads the effective UID of this process.
    let uid = unsafe { libc::geteuid() };
    if !metadata.is_dir() || metadata.uid() != uid || metadata.mode() & 0o022 != 0 {
        return Err(io::Error::other(
            "credential directory has unsafe ownership or permissions",
        ));
    }
    let path = directory.join("credentials.key");
    match read_key(&path, uid) {
        Err(error) if create && error.kind() == io::ErrorKind::NotFound => {}
        result => return result,
    }

    let mut key = [0; 32];
    getrandom::fill(&mut key).map_err(|error| io::Error::other(error.to_string()))?;
    let mut pending = tempfile::NamedTempFile::new_in(directory)?;
    pending.write_all(&key)?;
    pending.as_file().sync_all()?;
    match pending.persist_noclobber(&path) {
        Ok(_) => {
            File::open(directory)?.sync_all()?;
            Ok(key)
        }
        Err(error) if error.error.kind() == io::ErrorKind::AlreadyExists => read_key(&path, uid),
        Err(error) => Err(error.error),
    }
}

fn read_key(path: &Path, uid: u32) -> io::Result<[u8; 32]> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.uid() != uid || metadata.mode() & 0o077 != 0 {
        return Err(io::Error::other(
            "credential key has unsafe ownership or permissions",
        ));
    }
    let mut key = [0; 32];
    file.read_exact(&mut key)?;
    if file.read(&mut [0])? != 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "invalid credential key size",
        ));
    }
    Ok(key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn concurrent_key_creation_shares_one_private_key() {
        let directory = tempfile::tempdir().unwrap();
        let dir = directory.path();
        let keys = std::thread::scope(|scope| {
            let first = scope.spawn(|| load_key(dir, true).unwrap());
            let second = scope.spawn(|| load_key(dir, true).unwrap());
            [first.join().unwrap(), second.join().unwrap()]
        });
        assert_eq!(keys[0], keys[1]);
        assert_eq!(keys[0], load_key(dir, false).unwrap());
        let key_path = dir.join("credentials.key");
        let mode = fs::metadata(&key_path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
        fs::set_permissions(&key_path, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(load_key(dir, false).is_err());
    }
}
