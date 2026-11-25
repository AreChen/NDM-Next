use reqwest::{Url, header::HeaderValue};
use serde::{Deserialize, Serialize};

use crate::{Error, ProxyOptions, Result};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMode {
    #[default]
    Inherit,
    Direct,
    /// Use normal OS routing (including an active TUN), ignoring HTTP proxies.
    SystemRoute,
    /// Use only the locally configured DIRECT listener or bound TCP interface.
    TunDirect,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyRoute {
    Direct,
    ConfiguredProxy,
    SystemProxy,
    OsRoute,
    BoundInterface,
    LocalDirectListener,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TunEgress {
    LocalPort(std::num::NonZeroU16),
    Interface(String),
}

impl TunEgress {
    pub fn parse(value: &str) -> Result<Self> {
        if !value.is_empty() && value.bytes().all(|c| c.is_ascii_digit()) {
            return value
                .parse()
                .map(Self::LocalPort)
                .map_err(|_| Error::TunEgressUnavailable);
        }
        if cfg!(any(target_os = "linux", target_os = "macos"))
            && !value.is_empty()
            && value.len() <= 15
            && value
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'_' | b'.'))
        {
            return Ok(Self::Interface(value.to_owned()));
        }
        Err(Error::TunEgressUnavailable)
    }

    pub fn input_value(&self) -> String {
        match self {
            Self::LocalPort(port) => port.to_string(),
            Self::Interface(name) => name.clone(),
        }
    }

    pub fn route(&self) -> ProxyRoute {
        match self {
            Self::LocalPort(_) => ProxyRoute::LocalDirectListener,
            Self::Interface(_) => ProxyRoute::BoundInterface,
        }
    }
}

/// Uses the same destination matcher as reqwest's system-proxy feature.
pub(crate) enum Routing {
    Fixed(ProxyRoute),
    System(Box<hyper_util::client::proxy::matcher::Matcher>),
    CustomClient,
}

impl Routing {
    pub(crate) fn new(
        mode: ProxyMode,
        proxy: Option<&ProxyOptions>,
        egress: Option<&TunEgress>,
    ) -> Self {
        match mode {
            ProxyMode::SystemRoute => Self::Fixed(ProxyRoute::OsRoute),
            ProxyMode::TunDirect => egress.map_or(Self::CustomClient, |e| Self::Fixed(e.route())),
            ProxyMode::Direct => Self::Fixed(ProxyRoute::Direct),
            ProxyMode::Inherit if proxy.is_some() => Self::Fixed(ProxyRoute::ConfiguredProxy),
            ProxyMode::Inherit => Self::System(Box::new(
                hyper_util::client::proxy::matcher::Matcher::from_system(),
            )),
        }
    }

    pub(crate) fn for_url(&self, url: &Url) -> Option<ProxyRoute> {
        match self {
            Self::Fixed(route) => Some(*route),
            Self::System(matcher) => {
                let uri = url.as_str().parse().ok()?;
                Some(if matcher.intercept(&uri).is_some() {
                    ProxyRoute::SystemProxy
                } else {
                    ProxyRoute::Direct
                })
            }
            Self::CustomClient => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpStage {
    #[default]
    Preparing,
    RangeProbe,
    Redirect,
    Segment,
    FullDownload,
    IntegrityCheck,
    Publish,
}

/// Shared with the MCP process. It contains status and counters, never URLs,
/// request headers, server bodies, or proxy addresses and credentials.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HttpDiagnostics {
    pub stage: HttpStage,
    pub http_status: Option<u16>,
    pub range_probe_status: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub segment_failure: Option<Box<SegmentFailure>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range_mode: Option<HttpRangeMode>,
    pub redirect_count: u8,
    pub effective_proxy: Option<ProxyRoute>,
    pub failure: Option<DownloadFailure>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentIssue {
    Request,
    HttpStatus,
    RangeIgnored,
    ResourceChanged,
    EtagMismatch,
    ContentRangeMismatch,
    ShortBody,
    OversizedBody,
    LocalOffset,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SegmentFailure {
    pub index: usize,
    pub start: u64,
    pub end: u64,
    pub expected_total: u64,
    pub response_range: Option<(u64, u64, u64)>,
    pub content_length: Option<u64>,
    pub strong_etag: bool,
    pub etag_matches: Option<bool>,
    pub final_url_matches: Option<bool>,
    pub effective_proxy: Option<ProxyRoute>,
    pub retries: u32,
    pub redirects: u8,
    pub http_status: Option<u16>,
    pub stage: HttpStage,
    pub issue: SegmentIssue,
    pub error: DownloadFailure,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HttpRangeMode {
    StrongEtag,
    VerifiedSha256,
    ConnectionLimit,
    RangeIgnored,
    RangeRejected,
    InvalidContentRange,
    MissingValidator,
    SegmentFallback,
    RateLimited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadFailure {
    HttpStatus,
    InvalidResponse,
    ChecksumMismatch,
    Network,
    Timeout,
    Connection,
    InvalidHeader,
    InvalidProxy,
    TunEgressUnavailable,
    Filesystem,
    DownloadFailed,
}

impl DownloadFailure {
    pub(crate) fn from_error(error: &Error) -> Self {
        match error {
            Error::HttpStatus(_) => Self::HttpStatus,
            Error::InvalidHttpResponse => Self::InvalidResponse,
            Error::ChecksumMismatch => Self::ChecksumMismatch,
            Error::Network(error) if error.is_timeout() => Self::Timeout,
            Error::Network(error) if error.is_connect() => Self::Connection,
            Error::Network(_) => Self::Network,
            Error::InvalidHeader => Self::InvalidHeader,
            Error::TunEgressUnavailable => Self::TunEgressUnavailable,
            Error::InvalidProxy => Self::InvalidProxy,
            Error::Io(_) => Self::Filesystem,
            _ => Self::DownloadFailed,
        }
    }

    pub fn from_stored_reason(reason: &str) -> Self {
        match reason {
            "server rejected the download" => Self::HttpStatus,
            "server returned an inconsistent response" => Self::InvalidResponse,
            "download SHA-256 does not match the source" => Self::ChecksumMismatch,
            "TUN direct exit is missing or unsupported" => Self::TunEgressUnavailable,
            "network error" => Self::Network,
            "file or operating system error" => Self::Filesystem,
            _ => Self::DownloadFailed,
        }
    }

    pub const fn message(self) -> &'static str {
        match self {
            Self::HttpStatus => "server rejected the download",
            Self::InvalidResponse => "server returned an inconsistent response",
            Self::ChecksumMismatch => "download SHA-256 does not match the source",
            Self::Network => "network error",
            Self::Timeout => "network request timed out",
            Self::Connection => "could not connect to the server or proxy",
            Self::InvalidHeader => "download request contains an invalid header",
            Self::TunEgressUnavailable => "TUN direct exit is missing or unsupported",
            Self::InvalidProxy => "proxy configuration is invalid",
            Self::Filesystem => "file or operating system error",
            Self::DownloadFailed => "download failed",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TaskActivity {
    pub sampled_at_unix_ms: u64,
    pub idle_ms: u64,
    pub recent_bytes_per_second: Option<u64>,
}

pub fn validate_user_agent(value: &str) -> Result<()> {
    if value.trim().is_empty()
        || value.len() > 1_024
        || !value.is_ascii()
        || value.chars().any(char::is_control)
    {
        return Err(Error::InvalidHeader);
    }
    HeaderValue::from_str(value).map_err(|_| Error::InvalidHeader)?;
    Ok(())
}
