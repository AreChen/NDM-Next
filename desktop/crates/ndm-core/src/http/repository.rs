use base64::{Engine, engine::general_purpose::STANDARD};
use reqwest::{
    Client, Url,
    header::{self, HeaderMap},
};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

pub(super) type Sha256Bytes = [u8; 32];

fn https_host(url: &Url, domains: &[&str]) -> bool {
    url.scheme() == "https"
        && url.port().is_none()
        && url.host_str().is_some_and(|host| domains.contains(&host))
}

fn hf_resolve(url: &Url) -> bool {
    https_host(url, &["huggingface.co", "www.huggingface.co", "hf.co"])
        && url.path_segments().is_some_and(|mut parts| {
            parts.any(|part| part == "resolve") && parts.next().is_some() && parts.next().is_some()
        })
}

fn basename(path: &str) -> Option<String> {
    if path.contains(['?', '#']) || path.contains("://") || path.chars().any(char::is_control) {
        return None;
    }
    let name = path.rsplit(['/', '\\']).next()?.trim();
    (!matches!(name, "" | "." | "..") && name.len() <= 255).then(|| super::safe_filename(name))
}

pub(super) fn query_filename(url: &Url) -> Option<String> {
    url.query_pairs().find_map(|(key, value)| {
        ["filepath", "filename", "file_name"]
            .iter()
            .any(|known| key.eq_ignore_ascii_case(known))
            .then(|| basename(&value))
            .flatten()
    })
}

pub(super) fn source_filename(url: &Url) -> Option<String> {
    query_filename(url).or_else(|| hf_resolve(url).then(|| path_filename(url)).flatten())
}

/// Only an explicit filename parameter or a file-like path, never an arbitrary query value.
pub fn http_filename_hint(url: &Url) -> Option<String> {
    if !matches!(url.scheme(), "http" | "https") {
        return None;
    }
    query_filename(url).or_else(|| path_filename(url))
}

fn path_filename(url: &Url) -> Option<String> {
    let segment = url.path_segments()?.next_back()?;
    let name = basename(
        &percent_encoding::percent_decode_str(segment)
            .decode_utf8()
            .ok()?,
    )?;
    if hf_resolve(url) {
        return Some(name);
    }
    let (stem, extension) = name.rsplit_once('.')?;
    (!stem.is_empty()
        && !extension.is_empty()
        && extension.len() <= 16
        && extension.bytes().all(|c| c.is_ascii_alphanumeric()))
    .then_some(name)
}

// HF documents X-Linked-ETag on /resolve/ as the Git LFS SHA-256 for large files.
// Git blob IDs and the storage CDN's ETag are not content SHA-256s.
pub(super) fn hf_lfs_digest(url: &Url, headers: &HeaderMap) -> Option<Sha256Bytes> {
    if !hf_resolve(url) {
        return None;
    }
    let tag = headers.get("x-linked-etag")?.to_str().ok()?;
    let value = tag
        .strip_prefix('"')
        .and_then(|v| v.strip_suffix('"'))
        .unwrap_or(tag);
    hex::decode(value).ok()?.try_into().ok()
}

// RFC 9530 Repr-Digest covers the whole representation, even on a 206 response.
// Content-Digest on a 206 covers only the returned range.
pub(super) fn representation_digest(headers: &HeaderMap) -> Option<Sha256Bytes> {
    headers
        .get_all("repr-digest")
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|line| line.split(','))
        .find_map(|field| {
            let value = field.trim().strip_prefix("sha-256=:")?.strip_suffix(':')?;
            STANDARD.decode(value).ok()?.try_into().ok()
        })
}

fn modelscope_listing(source: &Url) -> Option<(Url, String)> {
    if !https_host(source, &["modelscope.cn", "www.modelscope.cn"])
        || !source.username().is_empty()
        || source.password().is_some()
    {
        return None;
    }
    let parts: Vec<_> = source.path_segments()?.collect();
    let (owner, repo, path, revision) = match parts.as_slice() {
        ["api", "v1", "models", owner, repo, "repo"] => {
            let mut path = None;
            let mut revision = None;
            for (key, value) in source.query_pairs() {
                match key.as_ref() {
                    "FilePath" if path.is_none() => path = Some(value.into_owned()),
                    "Revision" if revision.is_none() => revision = Some(value.into_owned()),
                    "FilePath" | "Revision" => return None,
                    _ => {}
                }
            }
            (
                *owner,
                *repo,
                path?,
                revision.unwrap_or_else(|| "master".into()),
            )
        }
        ["models", owner, repo, "resolve", revision, tail @ ..] if !tail.is_empty() => {
            let decode = |s: &str| {
                percent_encoding::percent_decode_str(s)
                    .decode_utf8()
                    .ok()
                    .map(|s| s.into_owned())
            };
            (*owner, *repo, decode(&tail.join("/"))?, decode(revision)?)
        }
        _ => return None,
    };
    if revision.is_empty()
        || revision.chars().any(char::is_control)
        || [owner, repo].iter().any(|s| {
            s.is_empty()
                || !s
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
        })
    {
        return None;
    }
    if path.is_empty()
        || path.len() > 4096
        || path.chars().any(char::is_control)
        || path.split('/').any(|p| matches!(p, "" | "." | ".."))
    {
        return None;
    }
    let mut listing = source.clone();
    listing.set_path(&format!("/api/v1/models/{owner}/{repo}/repo/files"));
    listing.set_query(None);
    listing.set_fragment(None);
    listing
        .query_pairs_mut()
        .append_pair("Root", path.rsplit_once('/').map_or("", |p| p.0))
        .append_pair("Revision", &revision)
        .append_pair("Recursive", "false");
    Some((listing, path))
}

#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Listing {
    code: u32,
    data: Entries,
}
#[derive(Deserialize)]
struct Entries {
    #[serde(rename = "Files")]
    files: Vec<Entry>,
}
#[derive(Deserialize)]
#[serde(rename_all = "PascalCase")]
struct Entry {
    path: String,
    size: Option<u64>,
    sha256: Option<String>,
    #[serde(rename = "Type")]
    kind: String,
}

fn listing_digest(body: &[u8], path: &str, total: u64) -> Option<Sha256Bytes> {
    let listing: Listing = serde_json::from_slice(body).ok()?;
    if listing.code != 200 {
        return None;
    }
    let mut matches = listing.data.files.iter().filter(|f| f.path == path);
    let file = matches.next()?;
    if matches.next().is_some() || file.kind != "blob" || file.size != Some(total) {
        return None;
    }
    hex::decode(file.sha256.as_deref()?).ok()?.try_into().ok()
}

pub(super) async fn modelscope_digest(
    source: &Url,
    total: u64,
    client: &Client,
    headers: &HeaderMap,
    cancel: &CancellationToken,
) -> Option<Sha256Bytes> {
    let (listing, path) = modelscope_listing(source)?;
    let mut query = client
        .get(listing)
        .header(header::ACCEPT, "application/json");
    for key in [header::AUTHORIZATION, header::COOKIE, header::USER_AGENT] {
        if let Some(value) = headers.get(&key) {
            query = query.header(key, value);
        }
    }
    let lookup = async {
        let mut response = super::send(client, query, cancel).await.ok()?;
        if response.status() != reqwest::StatusCode::OK
            || response.url().origin() != source.origin()
        {
            return None;
        }
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.ok()? {
            if body.len() + chunk.len() > 2 * 1024 * 1024 {
                return None;
            }
            body.extend_from_slice(&chunk);
        }
        listing_digest(&body, &path, total)
    };
    tokio::select! {
        _ = cancel.cancelled() => None,
        result = tokio::time::timeout(std::time::Duration::from_secs(5), lookup) => result.ok().flatten(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn repository_and_attachment_names_are_decoded_basenames() {
        for (url, expected) in [
            (
                "https://www.modelscope.cn/api/v1/models/A/B/repo?FilePath=docs%2FLICENSE",
                Some("LICENSE"),
            ),
            (
                "https://huggingface.co/A/B/resolve/main/LICENSE",
                Some("LICENSE"),
            ),
            (
                "https://www.modelscope.cn/api/v1/models/A/B/repo?FilePath=vae%2F%E6%A8%A1%E5%9E%8B.safetensors",
                Some("模型.safetensors"),
            ),
            (
                "https://huggingface.co/A/B/resolve/main/vae/model.safetensors?download=true",
                Some("model.safetensors"),
            ),
            (
                "https://example.test/download?filename=folder%5Cnotes.zip",
                Some("notes.zip"),
            ),
            ("https://example.test/repo?signature=private.bin", None),
            ("https://example.test/download?filename=%00bad.zip", None),
            ("https://example.test/download?filename=..", None),
        ] {
            assert_eq!(
                http_filename_hint(&Url::parse(url).unwrap()).as_deref(),
                expected
            );
        }
    }

    #[test]
    fn trusted_metadata_matches_exact_resource_revision_and_size() {
        let source = Url::parse("https://www.modelscope.cn/api/v1/models/Org/Repo/repo?Revision=dev%2Fv2&FilePath=models%2Fvae.bin&secret=hidden").unwrap();
        let (listing, path) = modelscope_listing(&source).unwrap();
        assert_eq!(
            listing.as_str(),
            "https://www.modelscope.cn/api/v1/models/Org/Repo/repo/files?Root=models&Revision=dev%2Fv2&Recursive=false"
        );
        let resolve = Url::parse("https://www.modelscope.cn/models/Org/Repo/resolve/dev%2Fv2/models/vae.bin?token=secret").unwrap();
        assert_eq!(
            modelscope_listing(&resolve),
            Some((listing.clone(), path.clone()))
        );
        let digest = "01".repeat(32);
        let listing = json!({"Code":200,"Data":{"Files":[{"Path":"other/vae.bin","Type":"blob","Size":4,"Sha256":"02".repeat(32)},{"Path":"models/vae.bin","Type":"blob","Size":4,"Sha256":digest}]}});
        let bytes = listing.to_string();
        assert_eq!(listing_digest(bytes.as_bytes(), &path, 4), Some([1; 32]));
        assert_eq!(listing_digest(bytes.as_bytes(), &path, 5), None);
        assert_eq!(listing_digest(bytes.as_bytes(), "vae.bin", 4), None);
        let mut spoof = source.clone();
        spoof.set_host(Some("modelscope.cn.example.test")).unwrap();
        assert!(modelscope_listing(&spoof).is_none());
        let hf = Url::parse("https://huggingface.co/org/repo/resolve/main/file").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-linked-etag", format!("\"{digest}\"").parse().unwrap());
        assert_eq!(hf_lfs_digest(&hf, &headers), Some([1; 32]));
        assert_eq!(hf_lfs_digest(&spoof, &headers), None);
        headers.insert("x-linked-etag", "W/\"tag\"".parse().unwrap());
        assert_eq!(hf_lfs_digest(&hf, &headers), None);
        headers.insert(
            "content-digest",
            format!("sha-256=:{}:", STANDARD.encode([1; 32]))
                .parse()
                .unwrap(),
        );
        assert_eq!(representation_digest(&headers), None);
    }
}
