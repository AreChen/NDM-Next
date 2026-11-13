use semver::Version;
use serde::Deserialize;
use std::time::Duration;

const RELEASES_API: &str = "https://api.github.com/repos/AreChen/NDM-Next/releases?per_page=100";
const MAX_RESPONSE: usize = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UpdateState {
    NotChecked,
    Checking,
    Current,
    Available(Version),
    Failed,
    StoreManaged,
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
}

fn find_update(data: &[u8], current: &Version) -> Result<Option<Version>, serde_json::Error> {
    let releases: Vec<Release> = serde_json::from_slice(data)?;
    Ok(releases
        .into_iter()
        .filter(|release| !release.draft && !release.prerelease)
        .filter_map(|release| {
            release
                .tag_name
                .strip_prefix("desktop-v")
                .and_then(|tag| Version::parse(tag).ok())
        })
        .filter(|version| version.pre.is_empty() && version.build.is_empty() && version > current)
        .max())
}

pub(crate) fn release_url(version: &Version) -> String {
    format!("https://github.com/AreChen/NDM-Next/releases/tag/desktop-v{version}")
}

pub(crate) async fn check() -> Result<Option<Version>, ()> {
    let current = Version::parse(env!("CARGO_PKG_VERSION")).map_err(|_| ())?;
    let client = reqwest::Client::builder()
        .user_agent(concat!("NDM-Next/", env!("CARGO_PKG_VERSION")))
        .timeout(Duration::from_secs(15))
        .connect_timeout(Duration::from_secs(8))
        .redirect(reqwest::redirect::Policy::limited(3))
        .build()
        .map_err(|_| ())?;
    let mut response = client.get(RELEASES_API).send().await.map_err(|_| ())?;
    if !response.status().is_success()
        || response
            .content_length()
            .is_some_and(|size| size > MAX_RESPONSE as u64)
    {
        return Err(());
    }
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE {
            return Err(());
        }
        body.extend_from_slice(&chunk);
    }
    find_update(&body, &current).map_err(|_| ())
}

#[cfg(windows)]
pub(crate) fn is_store_managed() -> bool {
    use windows_sys::Win32::{
        Foundation::APPMODEL_ERROR_NO_PACKAGE, Storage::Packaging::Appx::GetCurrentPackageFullName,
    };
    let mut size = 0;
    // SAFETY: A null output buffer with a valid length pointer queries package identity.
    unsafe {
        GetCurrentPackageFullName(&mut size, std::ptr::null_mut()) != APPMODEL_ERROR_NO_PACKAGE
    }
}

#[cfg(not(windows))]
pub(crate) fn is_store_managed() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ignores_extension_tags_and_non_stable_releases() {
        let data = r#"[
          {"tag_name":"v99.0.0","draft":false,"prerelease":false},
          {"tag_name":"desktop-v2.0.0","draft":true,"prerelease":false},
          {"tag_name":"desktop-v3.0.0-beta.1","draft":false,"prerelease":true},
          {"tag_name":"desktop-v1.4.0","draft":false,"prerelease":false},
          {"tag_name":"desktop-v1.3.5","draft":false,"prerelease":false},
          {"tag_name":"desktop-v1.0.0/evil","draft":false,"prerelease":false}
        ]"#;
        let current = semver::Version::parse("1.3.0").unwrap();
        assert_eq!(
            find_update(data.as_bytes(), &current)
                .unwrap()
                .unwrap()
                .to_string(),
            "1.4.0"
        );
        let current = semver::Version::parse("1.4.0").unwrap();
        assert!(find_update(data.as_bytes(), &current).unwrap().is_none());
    }
}
