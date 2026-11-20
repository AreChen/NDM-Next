use std::path::{Path, PathBuf};

fn legacy_download_directory() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        linux_home().join("Downloads")
    }
    #[cfg(not(target_os = "linux"))]
    {
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
}

/// Resolve the system's configured Downloads folder.
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
    #[cfg(target_os = "linux")]
    if let Ok(config) = std::fs::read_to_string(linux_config_directory().join("user-dirs.dirs"))
        && let Some(path) = linux_download_directory(&config, &linux_home())
    {
        return path;
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
    #[cfg(target_os = "linux")]
    {
        linux_base_directory("XDG_DATA_HOME", ".local/share").join("ndm-next")
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
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

#[cfg(target_os = "linux")]
fn linux_home() -> PathBuf {
    std::env::home_dir()
        .filter(|path| path.is_absolute())
        .expect("Linux requires an absolute user home directory")
}

#[cfg(target_os = "linux")]
fn linux_base_directory(variable: &str, default: &str) -> PathBuf {
    std::env::var_os(variable)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .unwrap_or_else(|| linux_home().join(default))
}

#[cfg(target_os = "linux")]
pub fn linux_config_directory() -> PathBuf {
    linux_base_directory("XDG_CONFIG_HOME", ".config")
}

#[cfg(target_os = "linux")]
fn linux_download_directory(config: &str, home: &Path) -> Option<PathBuf> {
    let encoded = config.lines().find_map(|line| {
        line.trim()
            .strip_prefix("XDG_DOWNLOAD_DIR=")?
            .trim()
            .strip_prefix('"')?
            .strip_suffix('"')
    })?;
    let mut expanded = String::new();
    let mut chars = encoded.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            let escaped = chars.next()?;
            if !matches!(escaped, '\\' | '"' | '$' | '`') {
                expanded.push('\\');
            }
            expanded.push(escaped);
        } else {
            expanded.push(ch);
        }
    }
    let path = if encoded == "$HOME" {
        home.to_owned()
    } else if encoded.starts_with("$HOME/") {
        let suffix = expanded.strip_prefix("$HOME/")?.trim_start_matches('/');
        home.join(suffix)
    } else {
        PathBuf::from(expanded)
    };
    path.is_absolute().then_some(path)
}

pub fn default_cache_directory() -> PathBuf {
    #[cfg(target_os = "macos")]
    {
        macos_home().join("Library/Caches/com.ndmnext.desktop")
    }
    #[cfg(target_os = "linux")]
    {
        linux_base_directory("XDG_CACHE_HOME", ".cache").join("ndm-next")
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
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

    #[cfg(target_os = "linux")]
    #[test]
    fn xdg_download_path_keeps_escaped_characters_and_rejects_relative_paths() {
        let home = Path::new("/home/test");
        let config = r#"XDG_DOWNLOAD_DIR="$HOME/Media Files/\$downloads""#;
        assert_eq!(
            super::linux_download_directory(config, home),
            Some(home.join("Media Files/$downloads"))
        );
        assert_eq!(
            super::linux_download_directory(r#"XDG_DOWNLOAD_DIR="/mnt/data""#, home),
            Some("/mnt/data".into())
        );
        assert!(super::linux_download_directory(r#"XDG_DOWNLOAD_DIR="relative""#, home).is_none());
    }
}
