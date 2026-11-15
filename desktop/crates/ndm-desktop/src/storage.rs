use crate::preferences::Preferences;
use ndm_core::{CacheReport, Scheduler};
use std::{
    fs, io,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

pub(crate) fn cleanup_due(last: Option<u64>, now: u64) -> bool {
    last.is_none_or(|last| {
        now.saturating_sub(last) >= 6 * 60 * 60 || last > now.saturating_add(300)
    })
}

pub(crate) fn validate_directory(path: &Path) -> io::Result<()> {
    if !path.is_absolute() {
        return Err(io::Error::other("directory must be absolute"));
    }
    fs::create_dir_all(path)?;
    let _probe = tempfile::Builder::new()
        .prefix(".ndm-next-write-check-")
        .tempfile_in(path)?;
    Ok(())
}

pub(crate) struct StorageChoice {
    downloads: PathBuf,
    cache: PathBuf,
    days: u32,
    follow_system: bool,
}

pub(crate) fn prepare_settings(
    downloads: PathBuf,
    cache: PathBuf,
    days: u32,
) -> io::Result<StorageChoice> {
    if days > 3650 {
        return Err(io::Error::other("cache retention exceeds 3650 days"));
    }
    validate_directory(&downloads)?;
    validate_directory(&cache)?;
    let system = ndm_core::system_download_directory();
    let follow_system = fs::canonicalize(&downloads)
        .ok()
        .zip(fs::canonicalize(system).ok())
        .is_some_and(|(left, right)| left == right);
    Ok(StorageChoice {
        downloads,
        cache,
        days,
        follow_system,
    })
}

pub(crate) fn apply_settings(
    mut preferences: Preferences,
    path: &Path,
    choice: StorageChoice,
) -> io::Result<Preferences> {
    preferences.use_system_download_dir = choice.follow_system;
    preferences.download_dir = choice.downloads;
    if preferences.cache_dir != choice.cache
        && !preferences
            .previous_cache_dirs
            .contains(&preferences.cache_dir)
    {
        preferences
            .previous_cache_dirs
            .push(preferences.cache_dir.clone());
    }
    preferences.cache_dir = choice.cache;
    preferences.cache_retention_days = choice.days;
    preferences.last_cache_cleanup = None;
    preferences.save(path)?;
    Ok(preferences)
}

fn reparse(metadata: &fs::Metadata) -> bool {
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

fn owned_update_name(name: &str) -> bool {
    name.strip_prefix("NDM-Next-Setup-")
        .and_then(|name| name.strip_suffix("-x64.exe"))
        .is_some_and(|version| semver::Version::parse(version).is_ok())
        || name
            .strip_prefix(".ndm-update-")
            .and_then(|name| name.strip_suffix(".part"))
            .is_some_and(|token| {
                !token.is_empty()
                    && token
                        .chars()
                        .all(|character| character.is_ascii_alphanumeric())
            })
}

fn update_cache(
    directory: &Path,
    cutoff: Option<SystemTime>,
    remove: bool,
    protected: bool,
    report: &mut CacheReport,
) {
    if !directory.is_absolute() {
        report.skipped_directories += 1;
        return;
    }
    let metadata = match fs::symlink_metadata(directory) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return,
        Err(_) => {
            report.skipped_directories += 1;
            return;
        }
    };
    if !metadata.is_dir() || reparse(&metadata) {
        report.skipped_directories += 1;
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        report.skipped_directories += 1;
        return;
    };
    for entry in entries {
        let Ok(entry) = entry else {
            report.skipped_directories += 1;
            continue;
        };
        if !owned_update_name(&entry.file_name().to_string_lossy()) {
            continue;
        }
        let Ok(metadata) = fs::symlink_metadata(entry.path()) else {
            continue;
        };
        if !metadata.is_file() || reparse(&metadata) {
            report.skipped_directories += 1;
            continue;
        }
        report.bytes = report.bytes.saturating_add(metadata.len());
        if protected {
            report.protected_bytes = report.protected_bytes.saturating_add(metadata.len());
            continue;
        }
        report.reclaimable_bytes = report.reclaimable_bytes.saturating_add(metadata.len());
        if remove
            && cutoff
                .is_none_or(|cutoff| metadata.modified().is_ok_and(|modified| modified <= cutoff))
        {
            match fs::remove_file(entry.path()) {
                Ok(()) => {
                    report.removed_bytes = report.removed_bytes.saturating_add(metadata.len());
                    report.removed_files += 1;
                }
                Err(_) => report.skipped_directories += 1,
            }
        }
    }
}

pub(crate) fn maintain(
    scheduler: &Scheduler,
    preferences: &Preferences,
    preferences_path: &Path,
    update_gate: &tokio::sync::Mutex<()>,
    automatic: bool,
    remove: bool,
) -> ndm_core::Result<CacheReport> {
    let remove = remove && (!automatic || preferences.cache_retention_days > 0);
    let cutoff = automatic.then(|| {
        SystemTime::now()
            .checked_sub(Duration::from_secs(
                u64::from(preferences.cache_retention_days) * 86400,
            ))
            .unwrap_or(SystemTime::UNIX_EPOCH)
    });
    let mut report = if remove {
        scheduler.clean_cache(cutoff)?
    } else {
        scheduler.cache_usage()?
    };
    // The updater holds this lease from download through installation, including Ready state.
    let update_lease = update_gate.try_lock();
    let protected = update_lease.is_err();
    let mut directories: Vec<_> = preferences
        .previous_cache_dirs
        .iter()
        .chain(std::iter::once(&preferences.cache_dir))
        .map(|root| root.join("updates"))
        .collect();
    if let Some(root) = preferences_path.parent() {
        directories.push(root.join("updates"));
    }
    // Canonical paths also deduplicate equivalent Windows spellings.
    let mut seen = std::collections::HashSet::new();
    for directory in directories {
        let identity = fs::canonicalize(&directory).unwrap_or_else(|_| directory.clone());
        if seen.insert(identity) {
            update_cache(&directory, cutoff, remove, protected, &mut report);
        }
    }
    report.bytes = report.bytes.saturating_sub(report.removed_bytes);
    report.reclaimable_bytes = report
        .reclaimable_bytes
        .saturating_sub(report.removed_bytes);
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::{apply_settings, cleanup_due, prepare_settings, update_cache};
    use crate::preferences::Preferences;
    use ndm_core::CacheReport;
    use std::{
        fs,
        time::{Duration, SystemTime},
    };

    #[test]
    fn directories_round_trip_and_old_cache_roots_are_remembered() {
        let temporary = tempfile::tempdir().unwrap();
        let path = temporary.path().join("preferences.json");
        let mut preferences = Preferences::default();
        let old_cache = preferences.cache_dir.clone();
        let downloads = temporary.path().join("保存目录");
        let cache = temporary.path().join("缓存目录");
        preferences = apply_settings(
            preferences,
            &path,
            prepare_settings(downloads.clone(), cache.clone(), 14).unwrap(),
        )
        .unwrap();
        assert!(!preferences.use_system_download_dir);
        assert_eq!(preferences.previous_cache_dirs, vec![old_cache]);
        assert_eq!(Preferences::load(&path).unwrap(), preferences);
        assert!(downloads.is_dir() && cache.is_dir());
        assert_eq!(fs::read_dir(&cache).unwrap().count(), 0);
        assert!(prepare_settings("relative".into(), cache, 7).is_err());
        assert_eq!(Preferences::load(&path).unwrap(), preferences);
    }

    #[test]
    fn update_cleanup_keeps_pending_installers_and_unrelated_files() {
        let temporary = tempfile::tempdir().unwrap();
        let installer = temporary.path().join("NDM-Next-Setup-0.1.6-x64.exe");
        let unrelated = temporary.path().join("personal.exe");
        fs::write(&installer, [1; 128]).unwrap();
        fs::write(&unrelated, b"keep").unwrap();
        let mut report = CacheReport::default();
        update_cache(temporary.path(), None, true, true, &mut report);
        assert_eq!(report.protected_bytes, 128);
        assert!(installer.exists());
        update_cache(
            temporary.path(),
            Some(SystemTime::UNIX_EPOCH),
            true,
            false,
            &mut report,
        );
        assert!(installer.exists());
        update_cache(
            temporary.path(),
            Some(SystemTime::now() + Duration::from_secs(2)),
            true,
            false,
            &mut report,
        );
        assert!(!installer.exists());
        assert_eq!(fs::read(unrelated).unwrap(), b"keep");
    }

    #[test]
    fn periodic_cleanup_is_infrequent_and_recovers_from_clock_changes() {
        assert!(cleanup_due(None, 10));
        assert!(!cleanup_due(Some(10), 100));
        assert!(cleanup_due(Some(10), 21610));
        assert!(cleanup_due(Some(1000), 10));
    }

    #[tokio::test]
    async fn updater_lease_protects_downloaded_installers_until_released() {
        let temporary = tempfile::tempdir().unwrap();
        let preferences = Preferences {
            cache_dir: temporary.path().join("cache"),
            ..Default::default()
        };
        let updates = preferences.cache_dir.join("updates");
        fs::create_dir_all(&updates).unwrap();
        let installer = updates.join("NDM-Next-Setup-0.1.6-x64.exe");
        fs::write(&installer, [1; 128]).unwrap();
        let store =
            std::sync::Arc::new(ndm_core::Store::open(&temporary.path().join("tasks.db")).unwrap());
        let scheduler = ndm_core::Scheduler::start(
            store,
            ndm_core::SchedulerOptions {
                auto_resume: false,
                ..Default::default()
            },
        )
        .unwrap();
        let gate = std::sync::Arc::new(tokio::sync::Mutex::new(()));
        let lease = gate.clone().lock_owned().await;
        let preferences_path = temporary.path().join("preferences.json");
        let report = super::maintain(
            &scheduler,
            &preferences,
            &preferences_path,
            &gate,
            false,
            true,
        )
        .unwrap();
        assert_eq!(report.protected_bytes, 128);
        assert!(installer.exists());
        drop(lease);
        let report = super::maintain(
            &scheduler,
            &preferences,
            &preferences_path,
            &gate,
            false,
            true,
        )
        .unwrap();
        assert_eq!(report.removed_bytes, 128);
        assert!(!installer.exists());
        scheduler.shutdown().await;
    }
}
