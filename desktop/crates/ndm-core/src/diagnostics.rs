use reqwest::{Url, header::HeaderValue};
use serde::{Deserialize, Serialize};

use crate::{Error, ProxyOptions, Result};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyMode {
    #[default]
    Inherit,
    Direct,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyRoute {
    Direct,
    ConfiguredProxy,
    SystemProxy,
}

/// Uses the same destination matcher as reqwest's system-proxy feature.
pub(crate) enum Routing {
    Fixed(ProxyRoute),
    System(Box<hyper_util::client::proxy::matcher::Matcher>),
    CustomClient,
}

impl Routing {
    pub(crate) fn new(mode: ProxyMode, proxy: Option<&ProxyOptions>) -> Self {
        match mode {
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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct HttpDiagnostics {
    pub stage: HttpStage,
    pub http_status: Option<u16>,
    pub range_probe_status: Option<u16>,
    pub redirect_count: u8,
    pub effective_proxy: Option<ProxyRoute>,
    pub failure: Option<DownloadFailure>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DownloadFailure {
    HttpStatus,
    InvalidResponse,
    Network,
    Timeout,
    Connection,
    InvalidHeader,
    InvalidProxy,
    Filesystem,
    DownloadFailed,
}

impl DownloadFailure {
    pub(crate) fn from_error(error: &Error) -> Self {
        match error {
            Error::HttpStatus(_) => Self::HttpStatus,
            Error::InvalidHttpResponse => Self::InvalidResponse,
            Error::Network(error) if error.is_timeout() => Self::Timeout,
            Error::Network(error) if error.is_connect() => Self::Connection,
            Error::Network(_) => Self::Network,
            Error::InvalidHeader => Self::InvalidHeader,
            Error::InvalidProxy => Self::InvalidProxy,
            Error::Io(_) => Self::Filesystem,
            _ => Self::DownloadFailed,
        }
    }

    pub fn from_stored_reason(reason: &str) -> Self {
        match reason {
            "server rejected the download" => Self::HttpStatus,
            "server returned an inconsistent response" => Self::InvalidResponse,
            "network error" => Self::Network,
            "file or operating system error" => Self::Filesystem,
            _ => Self::DownloadFailed,
        }
    }

    pub const fn message(self) -> &'static str {
        match self {
            Self::HttpStatus => "server rejected the download",
            Self::InvalidResponse => "server returned an inconsistent response",
            Self::Network => "network error",
            Self::Timeout => "network request timed out",
            Self::Connection => "could not connect to the server or proxy",
            Self::InvalidHeader => "download request contains an invalid header",
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
