//! Per-user Chromium native messaging registration on macOS and Linux.
use crate::{BROWSER_HOST_NAME, LEGACY_BROWSER_HOST_NAME};
use serde_json::Value;
use std::{fs, io, path::Path};

const HOST_NAMES: [&str; 2] = [BROWSER_HOST_NAME, LEGACY_BROWSER_HOST_NAME];
pub const DEFAULT_IDS: [&str; 3] = [
    "caegebfdojcdlkkiaiokeoadefdooamd",
    "hiblommiheggidolemmnnlocmocebckp",
    "cpapmfpgmpbpnjgibdgkfkdeblamiodm",
];
#[cfg(target_os = "macos")]
const BROWSERS: [&str; 4] = [
    "Google/Chrome",
    "Microsoft Edge",
    "BraveSoftware/Brave-Browser",
    "Chromium",
];
#[cfg(target_os = "linux")]
const BROWSERS: [&str; 4] = [
    "google-chrome",
    "microsoft-edge",
    "BraveSoftware/Brave-Browser",
    "chromium",
];

pub fn register_browser_hosts(executable: &Path, extra_ids: &[String]) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    let home = std::env::var_os("HOME").ok_or_else(|| io::Error::other("HOME is missing"))?;
    #[cfg(target_os = "macos")]
    let config = Path::new(&home).join("Library/Application Support");
    #[cfg(target_os = "linux")]
    let config = ndm_core::linux_config_directory();
    register_in(&config, executable, extra_ids)
}

fn register_in(config: &Path, executable: &Path, extra_ids: &[String]) -> io::Result<()> {
    let executable = fs::canonicalize(executable)?;
    let mut ids: Vec<String> = DEFAULT_IDS.iter().map(|id| (*id).to_owned()).collect();
    for id in extra_ids {
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    // Validate before touching any browser files.
    crate::native_manifest(BROWSER_HOST_NAME, &executable, &ids)?;
    for browser in BROWSERS {
        let directory = config.join(browser).join("NativeMessagingHosts");
        fs::create_dir_all(&directory)?;
        let destinations = HOST_NAMES.map(|name| directory.join(format!("{name}.json")));
        let mut browser_ids = ids.clone();
        for (name, destination) in HOST_NAMES.into_iter().zip(&destinations) {
            if let Ok(bytes) = fs::read(destination)
                && let Ok(old) = serde_json::from_slice::<Value>(&bytes)
                && old["name"] == name
                && old["type"] == "stdio"
                && let Some(origins) = old["allowed_origins"].as_array()
            {
                for origin in origins {
                    if let Some(id) = origin
                        .as_str()
                        .and_then(|origin| origin.strip_prefix("chrome-extension://"))
                        .and_then(|origin| origin.strip_suffix('/'))
                        && browser_ids.len() < 16
                        && id.len() == 32
                        && id.bytes().all(|byte| (b'a'..=b'p').contains(&byte))
                        && !browser_ids.iter().any(|known| known == id)
                    {
                        browser_ids.push(id.to_owned());
                    }
                }
            }
        }
        for (name, destination) in HOST_NAMES.into_iter().zip(destinations) {
            let manifest = crate::native_manifest(name, &executable, &browser_ids)?;
            let mut file = tempfile::NamedTempFile::new_in(&directory)?;
            serde_json::to_writer_pretty(&mut file, &manifest).map_err(io::Error::other)?;
            file.persist(destination).map_err(io::Error::other)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn registration_uses_actual_bundle_path_and_preserves_valid_developer_ids() {
        let home = tempfile::tempdir().unwrap();
        let old = home.path().join("Old App.app/Contents/MacOS/ndm-host");
        let new = home.path().join("OnesDM.app/Contents/MacOS/ndm-host");
        for path in [&old, &new] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"host").unwrap();
        }
        let extra = "abcdefghijklmnopabcdefghijklmnop".to_owned();
        let original = crate::native_manifest(
            LEGACY_BROWSER_HOST_NAME,
            &fs::canonicalize(&old).unwrap(),
            std::slice::from_ref(&extra),
        )
        .unwrap();
        for browser in BROWSERS {
            let path = home.path().join(browser).join("NativeMessagingHosts");
            fs::create_dir_all(&path).unwrap();
            fs::write(
                path.join(format!("{LEGACY_BROWSER_HOST_NAME}.json")),
                serde_json::to_vec(&original).unwrap(),
            )
            .unwrap();
        }
        register_in(home.path(), &new, &[]).unwrap();
        for browser in BROWSERS {
            for name in HOST_NAMES {
                let file = home
                    .path()
                    .join(browser)
                    .join("NativeMessagingHosts")
                    .join(format!("{name}.json"));
                let manifest: Value = serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
                assert_eq!(manifest["name"], name);
                for id in DEFAULT_IDS.into_iter().chain([extra.as_str()]) {
                    assert!(
                        manifest["allowed_origins"]
                            .as_array()
                            .unwrap()
                            .contains(&Value::String(format!("chrome-extension://{id}/")))
                    );
                }
                assert_eq!(
                    manifest["path"],
                    fs::canonicalize(&new).unwrap().to_str().unwrap()
                );
            }
        }
        assert!(register_in(home.path(), &new, &["*".into()]).is_err());
        assert!(!new.with_file_name("native-messaging-host.json").exists());
    }
}
