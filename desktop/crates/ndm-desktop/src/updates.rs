use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

const RELEASES_API: &str = "https://api.github.com/repos/AreChen/NDM-Next/releases?per_page=100";
const MAX_RESPONSE: usize = 2 * 1024 * 1024;
const MAX_INSTALLER: u64 = 256 * 1024 * 1024;

pub(crate) fn automatic_check_due(last: Option<u64>, now: u64) -> bool {
    last.is_none_or(|last| now < last || now - last >= 12 * 60 * 60)
}

// A cached version is only a hint. Installation after restart still requires
// fresh GitHub metadata and a trusted, verified installer download.
pub(crate) fn remembered_version(version: Option<&str>) -> Option<Version> {
    let version = Version::parse(version?).ok()?;
    let current = Version::parse(env!("CARGO_PKG_VERSION")).ok()?;
    (version > current && version.pre.is_empty() && version.build.is_empty()).then_some(version)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct UpdateRelease {
    pub version: Version,
    name: String,
    url: String,
    bytes: u64,
    sha256: [u8; 32],
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReadyUpdate {
    pub release: UpdateRelease,
    path: PathBuf,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UpdateState {
    NotChecked,
    Checking,
    Current,
    Available(UpdateRelease),
    Downloading,
    Ready(ReadyUpdate),
    Installing,
    Failed,
}

impl UpdateState {
    pub(crate) fn installation_requested(&self, requested: bool) -> bool {
        requested && matches!(self, Self::Ready(_))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InstallWait {
    Editor,
    Confirmation,
    Deleting,
    Media,
    Downloads,
    OtherInstaller,
    Storage,
}
#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    #[serde(default)]
    assets: Vec<Asset>,
}
#[derive(Deserialize)]
struct Asset {
    name: String,
    size: u64,
    browser_download_url: String,
    digest: Option<String>,
}

fn parse_digest(value: &str) -> Option<[u8; 32]> {
    let hex = value.strip_prefix("sha256:")?;
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    let mut digest = [0; 32];
    for (index, byte) in digest.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).ok()?;
    }
    Some(digest)
}
fn find_update(data: &[u8], current: &Version) -> Result<Option<UpdateRelease>, serde_json::Error> {
    let releases: Vec<Release> = serde_json::from_slice(data)?;
    Ok(releases
        .into_iter()
        .filter_map(|release| {
            if release.draft || release.prerelease {
                return None;
            }
            let version = Version::parse(release.tag_name.strip_prefix("desktop-v")?).ok()?;
            if !version.pre.is_empty() || !version.build.is_empty() || &version <= current {
                return None;
            }
            let name = format!("NDM-Next-Setup-{version}-x64.exe");
            let url = format!(
                "https://github.com/AreChen/NDM-Next/releases/download/desktop-v{version}/{name}"
            );
            let mut matching = release
                .assets
                .into_iter()
                .filter(|asset| asset.name == name);
            let asset = matching.next()?;
            if matching.next().is_some()
                || asset.browser_download_url != url
                || asset.size == 0
                || asset.size > MAX_INSTALLER
            {
                return None;
            }
            Some(UpdateRelease {
                version,
                name,
                url,
                bytes: asset.size,
                sha256: parse_digest(asset.digest.as_deref()?)?,
            })
        })
        .max_by(|a, b| a.version.cmp(&b.version)))
}
pub(crate) fn release_url(version: &Version) -> String {
    format!("https://github.com/AreChen/NDM-Next/releases/tag/desktop-v{version}")
}
fn trusted_download_url(url: &reqwest::Url) -> bool {
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && url.username().is_empty()
        && url.password().is_none()
        && matches!(
            url.host_str(),
            Some(
                "github.com"
                    | "api.github.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
}
fn client(proxy: Option<&ndm_core::ProxyOptions>) -> Result<reqwest::Client, ()> {
    let mut builder = reqwest::Client::builder()
        .user_agent(concat!("NDM-Next/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .read_timeout(Duration::from_secs(45))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if attempt.previous().len() >= 5 || !trusted_download_url(attempt.url()) {
                attempt.error("Invalid update redirect")
            } else {
                attempt.follow()
            }
        }));
    if let Some(options) = proxy {
        let mut proxy = reqwest::Proxy::all(&options.url).map_err(|_| ())?;
        if let Some(username) = &options.username {
            proxy = proxy.basic_auth(username, options.password.as_deref().unwrap_or(""));
        }
        builder = builder.proxy(proxy);
    }
    builder.build().map_err(|_| ())
}
pub(crate) async fn check(
    proxy: Option<ndm_core::ProxyOptions>,
) -> Result<Option<UpdateRelease>, ()> {
    let current = Version::parse(env!("CARGO_PKG_VERSION")).map_err(|_| ())?;
    check_version(proxy, &current).await
}
async fn check_version(
    proxy: Option<ndm_core::ProxyOptions>,
    current: &Version,
) -> Result<Option<UpdateRelease>, ()> {
    let mut response = client(proxy.as_ref())?
        .get(RELEASES_API)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|_| ())?;
    if response.status() != reqwest::StatusCode::OK
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
    find_update(&body, current).map_err(|_| ())
}
fn plain_file(metadata: &std::fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return false;
        }
    }
    metadata.is_file() && !metadata.is_symlink()
}
async fn receive_installer(
    mut response: reqwest::Response,
    release: UpdateRelease,
    directory: &Path,
) -> Result<ReadyUpdate, ()> {
    if response.status() != reqwest::StatusCode::OK
        || release.bytes == 0
        || release.bytes > MAX_INSTALLER
        || response
            .content_length()
            .is_some_and(|bytes| bytes != release.bytes)
    {
        return Err(());
    }
    tokio::fs::create_dir_all(directory).await.map_err(|_| ())?;
    let directory_metadata = tokio::fs::symlink_metadata(directory)
        .await
        .map_err(|_| ())?;
    if !directory_metadata.is_dir() || directory_metadata.is_symlink() {
        return Err(());
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if directory_metadata.file_attributes() & 0x400 != 0 {
            return Err(());
        }
    }
    let temporary = tempfile::Builder::new()
        .prefix(".ndm-update-")
        .suffix(".part")
        .tempfile_in(directory)
        .map_err(|_| ())?;
    let (file, temporary) = temporary.into_parts();
    let mut file = tokio::fs::File::from_std(file);
    let mut count = 0u64;
    let mut digest = Sha256::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| ())? {
        count = count.checked_add(chunk.len() as u64).ok_or(())?;
        if count > release.bytes {
            return Err(());
        }
        digest.update(&chunk);
        file.write_all(&chunk).await.map_err(|_| ())?;
    }
    if count != release.bytes || <[u8; 32]>::from(digest.finalize()) != release.sha256 {
        return Err(());
    }
    file.sync_all().await.map_err(|_| ())?;
    drop(file);
    let path = directory.join(&release.name);
    temporary.persist(&path).map_err(|_| ())?;
    Ok(ReadyUpdate { release, path })
}
pub(crate) async fn download(
    release: UpdateRelease,
    directory: PathBuf,
    proxy: Option<ndm_core::ProxyOptions>,
) -> Result<ReadyUpdate, ()> {
    let response = client(proxy.as_ref())?
        .get(&release.url)
        .timeout(Duration::from_secs(30 * 60))
        .send()
        .await
        .map_err(|_| ())?;
    receive_installer(response, release, &directory).await
}
async fn verify_cached(update: &ReadyUpdate) -> Result<(), ()> {
    let metadata = tokio::fs::symlink_metadata(&update.path)
        .await
        .map_err(|_| ())?;
    if !plain_file(&metadata) || metadata.len() != update.release.bytes {
        return Err(());
    }
    let mut file = tokio::fs::File::open(&update.path).await.map_err(|_| ())?;
    let mut digest = Sha256::new();
    let mut buffer = [0u8; 64 * 1024];
    let mut bytes = 0u64;
    loop {
        let read = file.read(&mut buffer).await.map_err(|_| ())?;
        if read == 0 {
            break;
        }
        bytes = bytes.checked_add(read as u64).ok_or(())?;
        if bytes > update.release.bytes {
            return Err(());
        }
        digest.update(&buffer[..read]);
    }
    if bytes != update.release.bytes || <[u8; 32]>::from(digest.finalize()) != update.release.sha256
    {
        return Err(());
    }
    Ok(())
}
pub(crate) async fn install(update: ReadyUpdate) -> Result<(), ()> {
    verify_cached(&update).await?;
    #[cfg(windows)]
    {
        use std::process::Stdio;
        let status = tokio::process::Command::new(&update.path)
            .args(["/S", "/UPDATE"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(false)
            .status()
            .await
            .map_err(|_| ())?;
        if status.success() { Ok(()) } else { Err(()) }
    }
    #[cfg(not(windows))]
    {
        Err(())
    }
}
#[cfg(test)]
mod tests;
