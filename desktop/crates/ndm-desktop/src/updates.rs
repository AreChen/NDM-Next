use semver::Version;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::{path::PathBuf, time::Duration};
use tokio::io::AsyncReadExt;

#[cfg(target_os = "macos")]
pub(crate) mod macos;

const RELEASES_API: &str = "https://api.github.com/repositories/1456128731/releases?per_page=100";
const REPOSITORY: &str = "AreChen/OnesDM";
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
    pub bytes: u64,
    sha256: [u8; 32],
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ReadyUpdate {
    pub release: UpdateRelease,
    path: PathBuf,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum UpdateFailure {
    Check(Option<u16>),
    Download,
    Storage,
    Integrity,
    Launch(Option<i32>),
    Installer(Option<i32>),
}
#[derive(Clone)]
pub(crate) struct UpdateTransfer {
    pub progress: ndm_core::DownloadProgress,
    pub expected: u64,
    pub verifying: Arc<AtomicBool>,
}
impl UpdateTransfer {
    pub fn new(expected: u64) -> Self {
        Self {
            progress: Default::default(),
            expected,
            verifying: Arc::new(AtomicBool::new(false)),
        }
    }
    pub fn is_verifying(&self) -> bool {
        self.verifying.load(Ordering::Acquire)
    }
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
    Failed(UpdateFailure),
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
fn asset_name(version: &Version) -> String {
    let branded = version >= &Version::new(0, 7, 0);
    let prefix = if branded { "OnesDM" } else { "NDM-Next" };
    if cfg!(target_os = "macos") {
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x64"
        };
        format!("{prefix}-{version}-macos-{arch}.dmg")
    } else if cfg!(target_os = "linux") {
        let arch = if cfg!(target_arch = "aarch64") {
            "arm64"
        } else {
            "x64"
        };
        format!("{prefix}-{version}-linux-{arch}.tar.gz")
    } else if branded {
        "OnesDM.exe".to_owned()
    } else {
        format!("NDM-Next-Setup-{version}-x64.exe")
    }
}
fn official_asset_url(url: &str, version: &Version, name: &str) -> bool {
    // GitHub canonicalizes repository casing, but tags and filenames are case-sensitive.
    let suffix = format!("/releases/download/desktop-v{version}/{name}");
    url.strip_prefix("https://github.com/")
        .and_then(|path| path.strip_suffix(&suffix))
        .is_some_and(|repo| {
            repo.eq_ignore_ascii_case(REPOSITORY) || repo.eq_ignore_ascii_case("AreChen/NDM-Next")
        })
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
            let name = asset_name(&version);
            let mut matching = release
                .assets
                .into_iter()
                .filter(|asset| asset.name == name);
            let asset = matching.next()?;
            if matching.next().is_some()
                || !official_asset_url(&asset.browser_download_url, &version, &name)
                || asset.size == 0
                || asset.size > MAX_INSTALLER
            {
                return None;
            }
            Some(UpdateRelease {
                version,
                name,
                url: asset.browser_download_url,
                bytes: asset.size,
                sha256: parse_digest(asset.digest.as_deref()?)?,
            })
        })
        .max_by(|a, b| a.version.cmp(&b.version)))
}
pub(crate) fn release_url(version: &Version) -> String {
    format!("https://github.com/{REPOSITORY}/releases/tag/desktop-v{version}")
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
        .user_agent(concat!("OnesDM/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30 * 60))
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
) -> Result<Option<UpdateRelease>, UpdateFailure> {
    let current =
        Version::parse(env!("CARGO_PKG_VERSION")).map_err(|_| UpdateFailure::Check(None))?;
    check_version(proxy, &current).await
}
async fn check_version(
    proxy: Option<ndm_core::ProxyOptions>,
    current: &Version,
) -> Result<Option<UpdateRelease>, UpdateFailure> {
    let mut response = client(proxy.as_ref())
        .map_err(|_| UpdateFailure::Check(None))?
        .get(RELEASES_API)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|_| UpdateFailure::Check(None))?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(UpdateFailure::Check(Some(response.status().as_u16())));
    }
    if response
        .content_length()
        .is_some_and(|size| size > MAX_RESPONSE as u64)
    {
        return Err(UpdateFailure::Check(None));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| UpdateFailure::Check(None))?
    {
        if body.len().saturating_add(chunk.len()) > MAX_RESPONSE {
            return Err(UpdateFailure::Check(None));
        }
        body.extend_from_slice(&chunk);
    }
    find_update(&body, current).map_err(|_| UpdateFailure::Check(None))
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
pub(crate) async fn download(
    release: UpdateRelease,
    directory: PathBuf,
    proxy: Option<ndm_core::ProxyOptions>,
    connections: usize,
    transfer: UpdateTransfer,
) -> Result<ReadyUpdate, UpdateFailure> {
    let client = client(proxy.as_ref()).map_err(|_| UpdateFailure::Download)?;
    download_with_client(release, directory, client, connections, transfer).await
}

async fn download_with_client(
    release: UpdateRelease,
    directory: PathBuf,
    client: reqwest::Client,
    connections: usize,
    transfer: UpdateTransfer,
) -> Result<ReadyUpdate, UpdateFailure> {
    if release.bytes == 0 || release.bytes > MAX_INSTALLER {
        return Err(UpdateFailure::Integrity);
    }
    tokio::fs::create_dir_all(&directory)
        .await
        .map_err(|_| UpdateFailure::Storage)?;
    let metadata = tokio::fs::symlink_metadata(&directory)
        .await
        .map_err(|_| UpdateFailure::Storage)?;
    if !metadata.is_dir() || metadata.is_symlink() {
        return Err(UpdateFailure::Storage);
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if metadata.file_attributes() & 0x400 != 0 {
            return Err(UpdateFailure::Storage);
        }
    }
    // A previous installation failure must not force another network transfer.
    // Cached files are trusted only against fresh release metadata and SHA-256.
    let cached = ReadyUpdate {
        path: directory.join(&release.name),
        release: release.clone(),
    };
    if verify_cached(&cached).await.is_ok() {
        return Ok(cached);
    }
    let staging = tempfile::Builder::new()
        .prefix(".ndm-update-")
        .suffix(".part")
        .tempdir_in(&directory)
        .map_err(|_| UpdateFailure::Storage)?;
    let request = ndm_core::NewTask {
        url: release.url.clone(),
        destination: staging.path().to_owned(),
        suggested_filename: Some(release.name.clone()),
        source_url: None,
        expected_size: Some(release.bytes),
        headers: Default::default(),
    };
    let path = ndm_core::download_http_with_client(
        &request,
        ndm_core::TaskId::new_v4(),
        &ndm_core::HttpOptions {
            max_connections: connections.clamp(1, 16),
            ..Default::default()
        },
        tokio_util::sync::CancellationToken::new(),
        transfer.progress.clone(),
        client,
    )
    .await
    .map_err(|error| match error {
        ndm_core::Error::Io(_) => UpdateFailure::Storage,
        ndm_core::Error::InvalidHttpResponse => UpdateFailure::Integrity,
        _ => UpdateFailure::Download,
    })?;
    transfer.verifying.store(true, Ordering::Release);
    let staged = ReadyUpdate { release, path };
    verify_cached(&staged)
        .await
        .map_err(|_| UpdateFailure::Integrity)?;
    let final_path = directory.join(&staged.release.name);
    tokio::fs::rename(&staged.path, &final_path)
        .await
        .map_err(|_| UpdateFailure::Storage)?;
    Ok(ReadyUpdate {
        release: staged.release,
        path: final_path,
    })
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
#[cfg(windows)]
async fn wait_for_installer_handoff(
    child: &mut tokio::process::Child,
    data_dir: &std::path::Path,
) -> Result<(), UpdateFailure> {
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| UpdateFailure::Launch(error.raw_os_error()))?
        {
            return if status.success() {
                Ok(())
            } else {
                Err(UpdateFailure::Installer(status.code()))
            };
        }
        if ndm_core::windows_installer_active(data_dir)
            .map_err(|error| UpdateFailure::Launch(error.raw_os_error()))?
        {
            // The installer waits for this process to exit before replacing files.
            // Return once its guard protects the handoff, so the GUI can exit
            // without retaining an update future that waits for the installer.
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
}

pub(crate) async fn install(
    update: ReadyUpdate,
    _data_dir: PathBuf,
    reservation: ndm_core::DesktopUpdateGuard,
) -> Result<(), UpdateFailure> {
    #[cfg(not(target_os = "macos"))]
    let _reservation = reservation;
    verify_cached(&update)
        .await
        .map_err(|_| UpdateFailure::Integrity)?;
    #[cfg(windows)]
    {
        use std::process::Stdio;
        let mut child = tokio::process::Command::new(&update.path)
            .args(["/S", "/UPDATE"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .kill_on_drop(false)
            .spawn()
            .map_err(|error| UpdateFailure::Launch(error.raw_os_error()))?;
        wait_for_installer_handoff(&mut child, &_data_dir).await
    }
    #[cfg(target_os = "macos")]
    {
        macos::install(update, _data_dir, reservation).await
    }
    #[cfg(target_os = "linux")]
    {
        let directory = update.path.parent().ok_or(UpdateFailure::Launch(None))?;
        let status = tokio::process::Command::new("xdg-open")
            .arg(directory)
            .status()
            .await
            .map_err(|error| UpdateFailure::Launch(error.raw_os_error()))?;
        if status.success() {
            Ok(())
        } else {
            Err(UpdateFailure::Installer(status.code()))
        }
    }
    #[cfg(not(any(windows, target_os = "macos", target_os = "linux")))]
    {
        Err(UpdateFailure::Launch(None))
    }
}
#[cfg(test)]
mod tests;
