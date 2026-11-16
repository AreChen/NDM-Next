use std::path::{Path, PathBuf};

fn legacy_download_directory() -> PathBuf {
    std::env::var_os(if cfg!(target_os = "macos") {
        "HOME"
    } else {
        "USERPROFILE"
    })
    .map(PathBuf::from)
    .filter(|path| path.is_absolute())
    .unwrap_or_else(std::env::temp_dir)
    .join("Downloads")
}

/// Resolve the configured Windows known folder, including redirected Downloads.
pub fn system_download_directory() -> PathBuf {
    #[cfg(windows)]
    {
        use std::{ffi::OsString, os::windows::ffi::OsStringExt};
        use windows_sys::Win32::{
            System::Com::CoTaskMemFree,
            UI::Shell::{FOLDERID_Downloads, KF_FLAG_DONT_VERIFY, SHGetKnownFolderPath},
        };
        let mut raw = std::ptr::null_mut();
        // SAFETY: valid GUID and out-pointer; the shell allocates this UTF-16 string.
        let status = unsafe {
            SHGetKnownFolderPath(
                &FOLDERID_Downloads,
                KF_FLAG_DONT_VERIFY as u32,
                std::ptr::null_mut(),
                &mut raw,
            )
        };
        let result = if status >= 0 && !raw.is_null() {
            // SAFETY: a successful call returns a NUL-terminated allocated string.
            unsafe {
                let mut length = 0;
                while *raw.add(length) != 0 {
                    length += 1;
                }
                Some(PathBuf::from(OsString::from_wide(
                    std::slice::from_raw_parts(raw, length),
                )))
            }
        } else {
            None
        };
        if !raw.is_null() {
            // SAFETY: this allocation belongs to the shell's COM task allocator.
            unsafe {
                CoTaskMemFree(raw.cast());
            }
        }
        if let Some(path) = result.filter(|path| path.is_absolute()) {
            return path;
        }
    }
    legacy_download_directory()
}

/// Older preferences had no flag distinguishing the automatic path from a custom one.
pub fn resolve_download_directory(
    saved: Option<&Path>,
    follow_system: Option<bool>,
) -> (PathBuf, bool) {
    resolve(
        saved,
        follow_system,
        &system_download_directory(),
        &legacy_download_directory(),
    )
}

fn resolve(
    saved: Option<&Path>,
    follow: Option<bool>,
    system: &Path,
    legacy: &Path,
) -> (PathBuf, bool) {
    let saved = saved.filter(|path| path.is_absolute());
    let automatic = match follow {
        Some(value) => value || saved.is_none(),
        None => saved.is_none_or(|path| path == legacy),
    };
    if automatic {
        (system.to_owned(), true)
    } else {
        (saved.unwrap().to_owned(), false)
    }
}

/// Persistent application state shared by the desktop and browser host.
pub fn application_data_directory() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        macos_home().join("Library/Application Support/NDM Next")
    }
    #[cfg(not(target_os = "macos"))]
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(std::env::temp_dir)
        .join("NDM Next")
}

#[cfg(target_os = "macos")]
pub(crate) fn macos_home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .expect("macOS requires an absolute HOME directory")
}

pub fn default_cache_directory() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        macos_home().join("Library/Caches/com.ndmnext.desktop")
    }
    #[cfg(not(target_os = "macos"))]
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(std::env::temp_dir)
        .join("NDM Next")
        .join("Cache")
}

#[cfg(test)]
mod tests {
    use super::{resolve, system_download_directory};
    use std::path::Path;

    #[test]
    fn redirected_downloads_replace_legacy_default_but_preserve_custom_paths() {
        let base = std::env::temp_dir();
        let system = base.join("redirected-downloads");
        let legacy = base.join("user-downloads");
        let custom = base.join("custom-downloads");
        assert_eq!(
            resolve(Some(&legacy), None, &system, &legacy),
            (system.clone(), true)
        );
        assert_eq!(
            resolve(Some(&custom), None, &system, &legacy),
            (custom, false)
        );
        assert_eq!(
            resolve(Some(&legacy), Some(false), &system, &legacy),
            (legacy.clone(), false)
        );
        assert_eq!(
            resolve(Some(Path::new("relative")), Some(false), &system, &legacy),
            (system.clone(), true)
        );
        assert_eq!(
            resolve(Some(&legacy), Some(true), &system, &legacy),
            (system, true)
        );
        assert!(system_download_directory().is_absolute());
    }
}
