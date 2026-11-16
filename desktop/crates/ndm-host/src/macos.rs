//! Per-user Chromium native messaging registration, outside the signed app bundle.
use serde_json::Value;
use std::{fs, io, path::Path};

pub const HOST_NAME: &str = "com.ndmnext.host";
pub const DEFAULT_IDS: [&str; 3] = [
    "caegebfdojcdlkkiaiokeoadefdooamd",
    "hiblommiheggidolemmnnlocmocebckp",
    "cpapmfpgmpbpnjgibdgkfkdeblamiodm",
];
const BROWSERS: [&str; 4] = [
    "Google/Chrome",
    "Microsoft Edge",
    "BraveSoftware/Brave-Browser",
    "Chromium",
];

pub fn register_browser_hosts(executable: &Path, extra_ids: &[String]) -> io::Result<()> {
    let home = std::env::var_os("HOME").ok_or_else(|| io::Error::other("HOME is missing"))?;
    register_in(Path::new(&home), executable, extra_ids)
}

fn register_in(home: &Path, executable: &Path, extra_ids: &[String]) -> io::Result<()> {
    let executable = fs::canonicalize(executable)?;
    let mut ids: Vec<String> = DEFAULT_IDS.iter().map(|id| (*id).to_owned()).collect();
    for id in extra_ids {
        if !ids.contains(id) {
            ids.push(id.clone());
        }
    }
    // Validate before touching any browser files.
    crate::native_manifest(HOST_NAME, &executable, &ids)?;
    for browser in BROWSERS {
        let directory = home
            .join("Library/Application Support")
            .join(browser)
            .join("NativeMessagingHosts");
        fs::create_dir_all(&directory)?;
        let destination = directory.join(format!("{HOST_NAME}.json"));
        let mut browser_ids = ids.clone();
        if let Ok(bytes) = fs::read(&destination)
            && let Ok(old) = serde_json::from_slice::<Value>(&bytes)
            && old["name"] == HOST_NAME
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
        let manifest = crate::native_manifest(HOST_NAME, &executable, &browser_ids)?;
        let mut file = tempfile::NamedTempFile::new_in(&directory)?;
        serde_json::to_writer_pretty(&mut file, &manifest).map_err(io::Error::other)?;
        file.persist(destination).map_err(io::Error::other)?;
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
        let new = home.path().join("NDM Next.app/Contents/MacOS/ndm-host");
        for path in [&old, &new] {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, b"host").unwrap();
        }
        let extra = "abcdefghijklmnopabcdefghijklmnop".to_owned();
        register_in(home.path(), &old, std::slice::from_ref(&extra)).unwrap();
        register_in(home.path(), &new, &[]).unwrap();
        for browser in BROWSERS {
            let file = home
                .path()
                .join("Library/Application Support")
                .join(browser)
                .join("NativeMessagingHosts/com.ndmnext.host.json");
            let manifest: Value = serde_json::from_slice(&fs::read(file).unwrap()).unwrap();
            for id in DEFAULT_IDS {
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
            assert!(
                manifest["allowed_origins"]
                    .as_array()
                    .unwrap()
                    .contains(&Value::String(format!("chrome-extension://{extra}/")))
            );
        }
        assert!(register_in(home.path(), &new, &["*".into()]).is_err());
        assert!(!new.with_file_name("native-messaging-host.json").exists());
    }
}
