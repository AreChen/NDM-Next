use std::collections::BTreeMap;

use ndm_core::{CHROME_USER_AGENT, TaskOptions, validate_user_agent};
use serde::{Deserialize, Serialize};

const IPHONE_SAFARI: &str = "Mozilla/5.0 (iPhone; CPU iPhone OS 18_6 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/18.6 Mobile/15E148 Safari/604.1";
const ANDROID_CHROME: &str = "Mozilla/5.0 (Linux; Android 15; Pixel 9) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/142.0.0.0 Mobile Safari/537.36";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UserAgentProfile {
    DesktopChrome,
    IphoneSafari,
    AndroidChrome,
}

impl UserAgentProfile {
    pub const fn value(self) -> &'static str {
        match self {
            Self::DesktopChrome => CHROME_USER_AGENT,
            Self::IphoneSafari => IPHONE_SAFARI,
            Self::AndroidChrome => ANDROID_CHROME,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum MediaType {
    Hls,
    Dash,
}

impl MediaType {
    pub fn mime(self) -> &'static str {
        match self {
            Self::Hls => "application/vnd.apple.mpegurl",
            Self::Dash => "application/dash+xml",
        }
    }

    pub fn task_options(self) -> TaskOptions {
        match self {
            Self::Hls => TaskOptions::Hls(Default::default()),
            Self::Dash => TaskOptions::Dash(Default::default()),
        }
    }
}

pub fn headers(
    user_agent: Option<&str>,
    profile: Option<UserAgentProfile>,
    referer: Option<&str>,
    accept_language: Option<&str>,
) -> Result<BTreeMap<String, String>, &'static str> {
    if user_agent.is_some() && profile.is_some() {
        return Err("Choose a User-Agent profile or provide a custom User-Agent.");
    }
    let mut headers = BTreeMap::new();
    if let Some(value) = user_agent.or_else(|| profile.map(UserAgentProfile::value)) {
        validate_user_agent(value).map_err(|_| "Invalid User-Agent.")?;
        headers.insert("user-agent".into(), value.to_owned());
    }
    if let Some(value) = referer {
        let mut url = super::web_url(value).ok_or("Referer must be an HTTP(S) page URL.")?;
        url.set_fragment(None);
        headers.insert("referer".into(), url.into());
    }
    if let Some(value) = accept_language {
        if value.trim().is_empty()
            || value.len() > 256
            || !value.bytes().all(|b| matches!(b, b' '..=b'~'))
        {
            return Err("Invalid Accept-Language.");
        }
        headers.insert("accept-language".into(), value.to_owned());
    }
    Ok(headers)
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamProbe {
    source: String,
    user_agent: Option<String>,
    user_agent_profile: Option<UserAgentProfile>,
    referer: Option<String>,
    accept_language: Option<String>,
}

pub fn probe_streams(probe: StreamProbe) -> Result<serde_json::Value, &'static str> {
    use serde_json::json;
    let source = super::web_url(&probe.source).ok_or("Provide an HTTP(S) page or stream URL.")?;
    let headers = headers(
        probe.user_agent.as_deref(),
        probe.user_agent_profile,
        probe.referer.as_deref(),
        probe.accept_language.as_deref(),
    )?;
    let user_agent = headers
        .get("user-agent")
        .map(String::as_str)
        .unwrap_or(CHROME_USER_AGENT);
    let known = playlist_kind(&source);
    let (page, candidates) = if let Some(kind) = known {
        (source.clone(), vec![(source, kind)])
    } else {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .map_err(|_| "Could not start the stream probe.")?
            .block_on(fetch_stream_links(source, user_agent, &headers))?
    };
    let links: Vec<_> = candidates
        .into_iter()
        .map(|(url, media_type)| {
            let referer = headers.get("referer").cloned().or_else(|| {
                (!page.eq(&url)).then(|| format!("{}/", page.origin().ascii_serialization()))
            });
            json!({
                "source":url.as_str(), "media_type":media_type, "user_agent":user_agent,
                "referer":referer, "accept_language":probe.accept_language
            })
        })
        .collect();
    Ok(
        json!({"status":if links.is_empty() {"browser_capture_needed"} else {"found"}, "items":links}),
    )
}

const PROBE_LIMIT: usize = 1024 * 1024;
const MAX_CANDIDATES: usize = 12;

async fn fetch_stream_links(
    source: reqwest::Url,
    user_agent: &str,
    hints: &BTreeMap<String, String>,
) -> Result<(reqwest::Url, Vec<(reqwest::Url, MediaType)>), &'static str> {
    use reqwest::header;
    use std::time::Duration;

    let referer = hints
        .get("referer")
        .and_then(|value| reqwest::Url::parse(value).ok())
        .filter(|url| url.scheme() != "https" || source.scheme() == "https");
    let secure_referer = referer.as_ref().is_some_and(|url| url.scheme() == "https");
    let client = reqwest::Client::builder()
        .user_agent(user_agent)
        .timeout(Duration::from_secs(12))
        .referer(false)
        .redirect(reqwest::redirect::Policy::custom(move |attempt| {
            if attempt.previous().len() >= 4
                || !matches!(attempt.url().scheme(), "http" | "https")
                || !attempt.url().username().is_empty()
                || attempt.url().password().is_some()
                || (secure_referer && attempt.url().scheme() == "http")
            {
                attempt.stop()
            } else {
                attempt.follow()
            }
        }))
        .build()
        .map_err(|_| "Could not configure the stream probe.")?;
    let mut request = client.get(source);
    if let Some(referer) = referer {
        request = request.header(
            header::REFERER,
            format!("{}/", referer.origin().ascii_serialization()),
        );
    }
    if let Some(language) = hints.get("accept-language") {
        request = request.header(header::ACCEPT_LANGUAGE, language);
    }
    let mut response = request
        .send()
        .await
        .map_err(|_| "Could not read the public page.")?;
    if !response.status().is_success() {
        return Err(
            "The source rejected the stream probe. Use the browser's captured stream if available.",
        );
    }
    let page = response.url().clone();
    let mime = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    let kind = match mime.as_str() {
        "application/vnd.apple.mpegurl"
        | "application/x-mpegurl"
        | "audio/mpegurl"
        | "audio/x-mpegurl" => Some(MediaType::Hls),
        "application/dash+xml" => Some(MediaType::Dash),
        _ => None,
    };
    if let Some(kind) = kind.or_else(|| playlist_kind(&page)) {
        return Ok((page.clone(), vec![(page, kind)]));
    }
    if (!mime.is_empty()
        && !mime.starts_with("text/")
        && !mime.contains("json")
        && !mime.ends_with("xml"))
        || response
            .content_length()
            .is_some_and(|n| n > PROBE_LIMIT as u64)
    {
        return Ok((page, Vec::new()));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| "Could not read the public page.")?
    {
        if body.len() + chunk.len() > PROBE_LIMIT {
            return Ok((page, Vec::new()));
        }
        body.extend_from_slice(&chunk);
    }
    let body = String::from_utf8_lossy(&body);
    if body
        .trim_start_matches('\u{feff}')
        .trim_start()
        .starts_with("#EXTM3U")
    {
        return Ok((page.clone(), vec![(page, MediaType::Hls)]));
    }
    let candidates = embedded_streams(&page, &body);
    Ok((page, candidates))
}

fn playlist_kind(url: &reqwest::Url) -> Option<MediaType> {
    let lower = url.path().to_ascii_lowercase();
    if lower.ends_with(".m3u8") {
        Some(MediaType::Hls)
    } else if lower.ends_with(".mpd") {
        Some(MediaType::Dash)
    } else {
        None
    }
}

fn embedded_streams(page: &reqwest::Url, body: &str) -> Vec<(reqwest::Url, MediaType)> {
    use std::{collections::HashSet, sync::LazyLock};
    static QUOTED_PLAYLIST: LazyLock<regex::Regex> = LazyLock::new(|| {
        regex::Regex::new(
            r#"(?i)["']([^"'<>\\\x00-\x20]+\.(?:m3u8|mpd)(?:\?[^"'<>\\\x00-\x20]*)?)["']"#,
        )
        .expect("valid playlist pattern")
    });
    let body = body.replace("\\/", "/").replace("&amp;", "&");
    let mut seen = HashSet::new();
    QUOTED_PLAYLIST
        .captures_iter(&body)
        .filter_map(|capture| {
            let url = page.join(capture.get(1)?.as_str()).ok()?;
            let url = super::web_url(url.as_str())?;
            let kind = playlist_kind(&url)?;
            seen.insert(url.clone()).then_some((url, kind))
        })
        .take(MAX_CANDIDATES)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{BufRead, BufReader, Write},
        net::TcpListener,
    };

    #[test]
    fn lightweight_probe_passes_mobile_hints_and_finds_observed_stream_links() {
        let server = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let page = format!("http://{}/replay", server.local_addr().unwrap());
        let fixture = std::thread::spawn(move || {
            for body in [
                r#"<video src='/playlist/phone.m3u8?variant=sd&amp;audio=1'></video>
                    <script>{"dash":"\/stream.mpd"}</script>"#,
                r#"<main id="app"></main><script src="/app.js"></script>"#,
            ] {
                let (stream, _) = server.accept().unwrap();
                stream
                    .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                    .unwrap();
                let mut stream = BufReader::new(stream);
                let request: Vec<_> = (&mut stream)
                    .lines()
                    .map(Result::unwrap)
                    .take_while(|line| !line.is_empty())
                    .collect();
                assert!(request[0].starts_with("GET /replay "));
                assert!(
                    request
                        .iter()
                        .any(|header| header == &format!("user-agent: {IPHONE_SAFARI}"))
                );
                assert!(
                    request
                        .iter()
                        .any(|header| header == "accept-language: zh-CN")
                );
                assert!(
                    request
                        .iter()
                        .any(|header| header == "referer: http://page.example/")
                );
                write!(stream.get_mut(), "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
            }
        });
        let probe = || {
            probe_streams(StreamProbe {
                source: page.clone(),
                user_agent: None,
                user_agent_profile: Some(UserAgentProfile::IphoneSafari),
                referer: Some("http://page.example/private?session=test".into()),
                accept_language: Some("zh-CN".into()),
            })
            .unwrap()
        };
        let result = probe();
        assert_eq!(result["status"], "found");
        let streams = result["items"].as_array().unwrap();
        assert_eq!(streams.len(), 2);
        assert_eq!(streams[0]["media_type"], "hls");
        assert!(
            streams[0]["source"]
                .as_str()
                .unwrap()
                .ends_with("/playlist/phone.m3u8?variant=sd&audio=1")
        );
        assert_eq!(streams[1]["media_type"], "dash");
        assert_eq!(probe()["status"], "browser_capture_needed");
        fixture.join().unwrap();
    }
}
