use reqwest::Url;
use roxmltree::{Document, Node, ParsingOptions};

use super::{DashAudio, DashCatalog, DashRepresentation};
use crate::{Error, Result};

pub(super) const MAX_SEGMENTS: usize = 16_384;
const MAX_REPRESENTATIONS: usize = 256;
const MAX_DURATION_MS: u64 = 604_800_000;
const MAX_TRACK_URL_BYTES: usize = 16 * 1024 * 1024;
const MAX_TOTAL_URL_BYTES: usize = 32 * 1024 * 1024;
const MAX_TOTAL_RESOURCES: usize = 65_536;
const NAMESPACE: &str = "urn:mpeg:dash:schema:mpd:2011";

#[derive(Clone, Debug)]
pub(super) struct Resource {
    pub url: Url,
    pub range: Option<(u64, u64)>,
    pub initialization: bool,
}

#[derive(Clone)]
pub(super) struct Track {
    pub resources: Vec<Resource>,
    pub kind: &'static str,
    pub group: usize,
    pub language: Option<String>,
    pub label: Option<String>,
    pub default: bool,
}

pub(super) struct Manifest {
    pub catalog: DashCatalog,
    pub video: Vec<Track>,
    pub audio: Vec<Track>,
}

fn named(node: Node<'_, '_>, name: &str) -> bool {
    node.is_element()
        && node.tag_name().name() == name
        && node
            .tag_name()
            .namespace()
            .is_none_or(|namespace| namespace == NAMESPACE)
}

fn child<'a, 'input>(node: Node<'a, 'input>, name: &str) -> Option<Node<'a, 'input>> {
    node.children().find(|node| named(*node, name))
}

fn attr<'a>(nodes: &[Node<'a, 'a>], name: &str) -> Option<&'a str> {
    nodes.iter().rev().find_map(|node| node.attribute(name))
}

fn integer(value: &str) -> Result<u64> {
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(Error::Dash);
    }
    value.parse().map_err(|_| Error::Dash)
}

/// DASH durations use ISO 8601 day/time units. Calendar months and years do not
/// have a fixed duration and are intentionally unsupported.
fn duration_ms(value: &str) -> Result<u64> {
    let mut remaining = value.strip_prefix('P').ok_or(Error::Dash)?;
    let mut in_time = false;
    let mut seconds = 0.0_f64;
    let mut count = 0;
    let mut previous_order = 0;
    while !remaining.is_empty() {
        if let Some(rest) = remaining.strip_prefix('T') {
            if in_time {
                return Err(Error::Dash);
            }
            in_time = true;
            remaining = rest;
            continue;
        }
        let end = remaining
            .find(|c: char| !c.is_ascii_digit() && c != '.')
            .ok_or(Error::Dash)?;
        let number = &remaining[..end];
        let unit = remaining.as_bytes()[end];
        let (multiplier, order) = match (in_time, unit) {
            (false, b'D') => (86_400.0, 1),
            (true, b'H') => (3600.0, 2),
            (true, b'M') => (60.0, 3),
            (true, b'S') => (1.0, 4),
            _ => return Err(Error::DashUnsupported),
        };
        if order <= previous_order || number.is_empty() || (number.contains('.') && unit != b'S') {
            return Err(Error::Dash);
        }
        let number: f64 = number.parse().map_err(|_| Error::Dash)?;
        if !number.is_finite() || number < 0.0 {
            return Err(Error::Dash);
        }
        seconds += number * multiplier;
        previous_order = order;
        remaining = &remaining[end + 1..];
        count += 1;
    }
    if count == 0 || !seconds.is_finite() || seconds * 1000.0 > MAX_DURATION_MS as f64 {
        return Err(Error::Dash);
    }
    Ok((seconds * 1000.0).round() as u64)
}

fn join(base: &Url, value: &str) -> Result<Url> {
    if value.len() > 65_536 || value.chars().any(char::is_control) {
        return Err(Error::Dash);
    }
    let url = base.join(value).map_err(|_| Error::Dash)?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(Error::DashUnsupported);
    }
    Ok(url)
}

fn base_url(nodes: &[Node<'_, '_>], manifest_url: &Url) -> Result<Url> {
    let mut url = manifest_url.clone();
    for node in nodes {
        if let Some(base) = child(*node, "BaseURL") {
            url = join(&url, base.text().ok_or(Error::Dash)?.trim())?;
        }
    }
    Ok(url)
}

fn range(value: Option<&str>) -> Result<Option<(u64, u64)>> {
    value
        .map(|value| {
            let (start, end) = value.split_once('-').ok_or(Error::Dash)?;
            let (start, end) = (integer(start)?, integer(end)?);
            if end < start
                || end
                    .checked_sub(start)
                    .and_then(|length| length.checked_add(1))
                    .is_none()
            {
                return Err(Error::Dash);
            }
            Ok((start, end))
        })
        .transpose()
}

fn substitute(template: &str, id: &str, bandwidth: u64, number: u64, time: u64) -> Result<String> {
    let mut output = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('$') {
        output.push_str(&rest[..start]);
        rest = &rest[start + 1..];
        if let Some(after) = rest.strip_prefix('$') {
            output.push('$');
            rest = after;
            continue;
        }
        let end = rest.find('$').ok_or(Error::Dash)?;
        let token = &rest[..end];
        let (name, formatting) = token
            .split_once('%')
            .map_or((token, None), |(name, format)| (name, Some(format)));
        if name == "RepresentationID" && formatting.is_none() {
            if id.is_empty() {
                return Err(Error::Dash);
            }
            output.push_str(id);
        } else {
            let value = match name {
                "Bandwidth" => bandwidth,
                "Number" => number,
                "Time" => time,
                _ => return Err(Error::DashUnsupported),
            };
            let width = if let Some(format) = formatting {
                let width = format
                    .strip_prefix('0')
                    .and_then(|value| value.strip_suffix('d'))
                    .ok_or(Error::DashUnsupported)?;
                usize::try_from(integer(width)?).map_err(|_| Error::Dash)?
            } else {
                0
            };
            if width > 20 {
                return Err(Error::Dash);
            }
            output.push_str(&format!("{value:0width$}"));
        }
        rest = &rest[end + 1..];
        if output.len() > 65_536 {
            return Err(Error::Dash);
        }
    }
    output.push_str(rest);
    Ok(output)
}

fn timeline(nodes: &[Node<'_, '_>], period_ms: Option<u64>) -> Result<(Vec<u64>, Option<u64>)> {
    let timescale = attr(nodes, "timescale")
        .map(integer)
        .transpose()?
        .unwrap_or(1);
    if timescale == 0 {
        return Err(Error::Dash);
    }
    let offset = attr(nodes, "presentationTimeOffset")
        .map(integer)
        .transpose()?
        .unwrap_or(0);
    let period_end = period_ms
        .map(|duration| {
            duration
                .checked_mul(timescale)
                .and_then(|value| value.checked_add(999))
                .map(|value| value / 1000)
                .and_then(|duration| offset.checked_add(duration))
                .ok_or(Error::Dash)
        })
        .transpose()?;
    let mut starts = Vec::new();
    let mut end_time = offset;
    if let Some(timeline) = nodes
        .iter()
        .rev()
        .find_map(|node| child(*node, "SegmentTimeline"))
    {
        let items: Vec<_> = timeline
            .children()
            .filter(|node| named(*node, "S"))
            .collect();
        if items.is_empty() || items.len() > MAX_SEGMENTS {
            return Err(Error::Dash);
        }
        for (index, item) in items.iter().enumerate() {
            let start = item
                .attribute("t")
                .map(integer)
                .transpose()?
                .unwrap_or(end_time);
            let duration = integer(item.attribute("d").ok_or(Error::Dash)?)?;
            if duration == 0 || (index > 0 && start < end_time) {
                return Err(Error::Dash);
            }
            if index == 0 && start != offset {
                return Err(Error::DashUnsupported);
            }
            let repeat = item
                .attribute("r")
                .unwrap_or("0")
                .parse::<i64>()
                .map_err(|_| Error::Dash)?;
            let count = if repeat >= 0 {
                u64::try_from(repeat)
                    .map_err(|_| Error::Dash)?
                    .checked_add(1)
                    .ok_or(Error::Dash)?
            } else if repeat == -1 {
                let end = items
                    .get(index + 1)
                    .and_then(|next| next.attribute("t"))
                    .map(integer)
                    .transpose()?
                    .or(period_end)
                    .ok_or(Error::DashUnsupported)?;
                let span = end
                    .checked_sub(start)
                    .filter(|span| *span > 0)
                    .ok_or(Error::Dash)?;
                span.checked_add(duration - 1).ok_or(Error::Dash)? / duration
            } else {
                return Err(Error::Dash);
            };
            if count > MAX_SEGMENTS as u64
                || starts.len().saturating_add(count as usize) > MAX_SEGMENTS
            {
                return Err(Error::Dash);
            }
            for iteration in 0..count {
                let time = duration
                    .checked_mul(iteration)
                    .and_then(|delta| start.checked_add(delta))
                    .ok_or(Error::Dash)?;
                if period_end.is_some_and(|end| time >= end) {
                    return Err(Error::Dash);
                }
                starts.push(time);
            }
            end_time = duration
                .checked_mul(count)
                .and_then(|delta| start.checked_add(delta))
                .ok_or(Error::Dash)?;
        }
    } else {
        let duration = integer(attr(nodes, "duration").ok_or(Error::DashUnsupported)?)?;
        let span = period_end
            .ok_or(Error::DashUnsupported)?
            .checked_sub(offset)
            .ok_or(Error::Dash)?;
        if duration == 0 || span == 0 {
            return Err(Error::Dash);
        }
        let count = span.checked_add(duration - 1).ok_or(Error::Dash)? / duration;
        if count > MAX_SEGMENTS as u64 {
            return Err(Error::Dash);
        }
        for index in 0..count {
            starts.push(
                index
                    .checked_mul(duration)
                    .and_then(|time| offset.checked_add(time))
                    .ok_or(Error::Dash)?,
            );
        }
        end_time = offset.checked_add(span).ok_or(Error::Dash)?;
    }
    let length = end_time
        .checked_sub(offset)
        .and_then(|time| time.checked_mul(1000))
        .ok_or(Error::Dash)?
        / timescale;
    if length > MAX_DURATION_MS {
        return Err(Error::Dash);
    }
    Ok((starts, Some(period_ms.unwrap_or(length))))
}

fn resources(
    hierarchy: &[Node<'_, '_>],
    base: &Url,
    id: &str,
    bandwidth: u64,
    period_ms: Option<u64>,
) -> Result<(Vec<Resource>, Option<u64>)> {
    let kind = hierarchy
        .iter()
        .rev()
        .find_map(|node| {
            node.children().find(|node| {
                named(*node, "SegmentTemplate")
                    || named(*node, "SegmentList")
                    || named(*node, "SegmentBase")
            })
        })
        .ok_or(Error::DashUnsupported)?
        .tag_name()
        .name();
    if kind == "SegmentBase" {
        return Err(Error::DashUnsupported);
    }
    let nodes: Vec<_> = hierarchy
        .iter()
        .filter_map(|node| child(*node, kind))
        .collect();
    if nodes.iter().any(|node| {
        node.attribute("index").is_some()
            || node.attribute("eptDelta").is_some()
            || node.attribute("pdDelta").is_some()
    }) {
        return Err(Error::DashUnsupported);
    }
    if kind == "SegmentTemplate" {
        let (starts, duration) = timeline(&nodes, period_ms)?;
        let start_number = attr(&nodes, "startNumber")
            .map(integer)
            .transpose()?
            .unwrap_or(1);
        let init = attr(&nodes, "initialization").ok_or(Error::DashUnsupported)?;
        let template = attr(&nodes, "media").ok_or(Error::DashUnsupported)?;
        let mut resources = vec![Resource {
            url: join(base, &substitute(init, id, bandwidth, start_number, 0)?)?,
            range: None,
            initialization: true,
        }];
        let mut url_bytes = resources[0].url.as_str().len();
        for (index, time) in starts.iter().enumerate() {
            let number = start_number.checked_add(index as u64).ok_or(Error::Dash)?;
            let url = join(base, &substitute(template, id, bandwidth, number, *time)?)?;
            url_bytes = url_bytes.saturating_add(url.as_str().len());
            if url_bytes > MAX_TRACK_URL_BYTES {
                return Err(Error::Dash);
            }
            resources.push(Resource {
                url,
                range: None,
                initialization: false,
            });
        }
        return Ok((resources, duration));
    }
    let init = nodes
        .iter()
        .rev()
        .find_map(|node| child(*node, "Initialization"))
        .ok_or(Error::DashUnsupported)?;
    let mut resources = vec![Resource {
        url: join(base, init.attribute("sourceURL").unwrap_or(""))?,
        range: range(init.attribute("range"))?,
        initialization: true,
    }];
    let mut url_bytes = resources[0].url.as_str().len();
    let list = nodes
        .iter()
        .rev()
        .find(|node| child(**node, "SegmentURL").is_some())
        .ok_or(Error::Dash)?;
    for segment in list.children().filter(|node| named(*node, "SegmentURL")) {
        if resources.len() > MAX_SEGMENTS {
            return Err(Error::Dash);
        }
        let url = join(base, segment.attribute("media").unwrap_or(""))?;
        url_bytes = url_bytes.saturating_add(url.as_str().len());
        if url_bytes > MAX_TRACK_URL_BYTES {
            return Err(Error::Dash);
        }
        resources.push(Resource {
            url,
            range: range(segment.attribute("mediaRange"))?,
            initialization: false,
        });
    }
    if resources.len() < 2 {
        return Err(Error::Dash);
    }
    let duration = if nodes
        .iter()
        .any(|node| child(*node, "SegmentTimeline").is_some())
    {
        let (starts, duration) = timeline(&nodes, period_ms)?;
        if starts.len() + 1 != resources.len() {
            return Err(Error::Dash);
        }
        duration
    } else {
        period_ms
    };
    Ok((resources, duration))
}

pub(super) fn parse(bytes: &[u8], url: &Url) -> Result<Manifest> {
    if bytes.len() > super::MAX_MANIFEST_BYTES {
        return Err(Error::Dash);
    }
    let text = std::str::from_utf8(bytes).map_err(|_| Error::Dash)?;
    let document = Document::parse_with_options(
        text,
        ParsingOptions {
            allow_dtd: false,
            nodes_limit: 100_000,
            entity_resolver: None,
        },
    )
    .map_err(|_| Error::Dash)?;
    let root = document.root_element();
    if !named(root, "MPD") {
        return Err(Error::Dash);
    }
    if root.attribute("type") == Some("dynamic") {
        return Err(Error::DashDynamicUnsupported);
    }
    if root.attribute("type").is_some_and(|kind| kind != "static") {
        return Err(Error::Dash);
    }
    if document
        .descendants()
        .any(|node| node.is_element() && node.tag_name().name() == "ContentProtection")
    {
        return Err(Error::DashDrmUnsupported);
    }
    if document.descendants().any(|node| {
        node.attributes()
            .any(|attr| attr.namespace() == Some("http://www.w3.org/1999/xlink"))
    }) {
        return Err(Error::DashUnsupported);
    }
    let periods: Vec<_> = root
        .children()
        .filter(|node| named(*node, "Period"))
        .collect();
    if periods.len() != 1 {
        return Err(Error::DashUnsupported);
    }
    let period = periods[0];
    if period
        .attribute("start")
        .map(duration_ms)
        .transpose()?
        .unwrap_or(0)
        != 0
    {
        return Err(Error::DashUnsupported);
    }
    let period_ms = period
        .attribute("duration")
        .or(root.attribute("mediaPresentationDuration"))
        .map(duration_ms)
        .transpose()?;
    let mut catalog = DashCatalog {
        representations: Vec::new(),
        audio_tracks: Vec::new(),
        duration_ms: period_ms,
    };
    let mut video = Vec::new();
    let mut audio = Vec::new();
    let mut total_url_bytes = 0_usize;
    let mut total_resources = 0_usize;
    for (group, adaptation) in period
        .children()
        .filter(|node| named(*node, "AdaptationSet"))
        .enumerate()
    {
        let role_main = adaptation
            .children()
            .any(|node| named(node, "Role") && node.attribute("value") == Some("main"));
        for representation in adaptation
            .children()
            .filter(|node| named(*node, "Representation"))
        {
            if video.len() + audio.len() >= MAX_REPRESENTATIONS {
                return Err(Error::Dash);
            }
            let hierarchy = [root, period, adaptation, representation];
            let mime = attr(&hierarchy, "mimeType").unwrap_or("");
            let kind = match attr(&hierarchy, "contentType")
                .unwrap_or_else(|| mime.split('/').next().unwrap_or(""))
            {
                "video" => "video",
                "audio" => "audio",
                _ => continue,
            };
            if !mime.is_empty() && !matches!(mime, "video/mp4" | "audio/mp4" | "application/mp4") {
                return Err(Error::DashUnsupported);
            }
            let id = representation.attribute("id").unwrap_or("");
            if id.len() > 512 {
                return Err(Error::Dash);
            }
            let bandwidth = representation
                .attribute("bandwidth")
                .map(integer)
                .transpose()?
                .unwrap_or(0);
            let base = base_url(&hierarchy, url)?;
            let (resources, duration) = resources(&hierarchy, &base, id, bandwidth, period_ms)?;
            total_resources = total_resources.saturating_add(resources.len());
            total_url_bytes = total_url_bytes.saturating_add(
                resources
                    .iter()
                    .map(|resource| resource.url.as_str().len())
                    .sum::<usize>(),
            );
            if total_resources > MAX_TOTAL_RESOURCES || total_url_bytes > MAX_TOTAL_URL_BYTES {
                return Err(Error::Dash);
            }
            catalog.duration_ms = catalog.duration_ms.into_iter().chain(duration).max();
            let language = attr(&hierarchy, "lang").map(str::to_owned);
            let label = hierarchy
                .iter()
                .rev()
                .find_map(|node| child(*node, "Label").and_then(|node| node.text()))
                .map(str::to_owned);
            let codecs = attr(&hierarchy, "codecs").map(str::to_owned);
            if language.as_ref().is_some_and(|value| value.len() > 256)
                || label.as_ref().is_some_and(|value| value.len() > 1024)
                || codecs.as_ref().is_some_and(|value| value.len() > 256)
            {
                return Err(Error::Dash);
            }
            let track = Track {
                resources,
                kind,
                group,
                language: language.clone(),
                label: label.clone(),
                default: role_main,
            };
            if kind == "video" {
                let width = attr(&hierarchy, "width").map(integer).transpose()?;
                let height = attr(&hierarchy, "height").map(integer).transpose()?;
                catalog.representations.push(DashRepresentation {
                    index: video.len(),
                    id: id.to_owned(),
                    bandwidth,
                    resolution: width.zip(height),
                    codecs,
                });
                video.push(track);
            } else {
                catalog.audio_tracks.push(DashAudio {
                    index: audio.len(),
                    group,
                    id: id.to_owned(),
                    language,
                    label,
                    bandwidth,
                    codecs,
                    default: role_main,
                });
                audio.push(track);
            }
        }
    }
    if video.is_empty() && audio.is_empty() {
        return Err(Error::DashUnsupported);
    }
    // Choose one rendition in the preferred language/group by default, rather
    // than treating every bitrate of the same audio as a distinct soundtrack.
    let preferred_group = audio
        .iter()
        .find(|track| track.default)
        .or_else(|| audio.first())
        .map(|track| track.group);
    let default_audio = catalog
        .audio_tracks
        .iter()
        .filter(|track| Some(audio[track.index].group) == preferred_group)
        .max_by_key(|track| track.bandwidth)
        .map(|track| track.index);
    for track in &mut catalog.audio_tracks {
        track.default = Some(track.index) == default_audio;
    }
    for (index, track) in audio.iter_mut().enumerate() {
        track.default = Some(index) == default_audio;
    }
    Ok(Manifest {
        catalog,
        video,
        audio,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mpd(body: &str) -> String {
        format!(
            "<MPD xmlns=\"{NAMESPACE}\" type=\"static\" mediaPresentationDuration=\"PT3S\"><Period>{body}</Period></MPD>"
        )
    }

    #[test]
    fn inherited_template_and_relative_base_urls_expand_real_resources() {
        let document = mpd(
            "<BaseURL>media/</BaseURL><AdaptationSet mimeType=\"video/mp4\"><SegmentTemplate timescale=\"10\" initialization=\"$RepresentationID$/init.mp4\" media=\"$RepresentationID$/seg-$Number%03d$.m4s\" startNumber=\"0\"><SegmentTimeline><S t=\"0\" d=\"10\" r=\"2\"/></SegmentTimeline></SegmentTemplate><Representation id=\"v\" bandwidth=\"100\" width=\"160\" height=\"90\"/></AdaptationSet>",
        );
        let parsed = parse(
            document.as_bytes(),
            &Url::parse("https://example.test/folder/manifest.mpd").unwrap(),
        )
        .unwrap();
        assert_eq!(
            parsed.video[0].resources[0].url.as_str(),
            "https://example.test/folder/media/v/init.mp4"
        );
        assert_eq!(
            parsed.video[0].resources[3].url.as_str(),
            "https://example.test/folder/media/v/seg-002.m4s"
        );
        assert_eq!(parsed.catalog.duration_ms, Some(3000));
        assert_eq!(
            parsed.catalog.representations[0].resolution,
            Some((160, 90))
        );
    }

    #[test]
    fn time_templates_support_bounded_negative_repeat() {
        let document = mpd(
            "<AdaptationSet mimeType=\"video/mp4\"><SegmentTemplate timescale=\"1000\" initialization=\"init\" media=\"segment-$Time$\"><SegmentTimeline><S t=\"0\" d=\"1000\" r=\"-1\"/></SegmentTimeline></SegmentTemplate><Representation id=\"v\"/></AdaptationSet>",
        );
        let parsed = parse(
            document.as_bytes(),
            &Url::parse("https://example.test/m.mpd").unwrap(),
        )
        .unwrap();
        assert_eq!(parsed.video[0].resources.len(), 4);
        assert_eq!(parsed.video[0].resources[3].url.path(), "/segment-2000");
    }

    #[test]
    fn segment_list_preserves_byte_ranges() {
        let document = mpd(
            "<AdaptationSet mimeType=\"audio/mp4\" lang=\"en\"><Representation id=\"a\"><BaseURL>audio.mp4</BaseURL><SegmentList><Initialization range=\"0-99\"/><SegmentURL mediaRange=\"100-299\"/></SegmentList></Representation></AdaptationSet>",
        );
        let parsed = parse(
            document.as_bytes(),
            &Url::parse("https://example.test/m.mpd").unwrap(),
        )
        .unwrap();
        assert_eq!(parsed.audio[0].resources[1].range, Some((100, 299)));
        assert_eq!(parsed.audio[0].resources[1].url.path(), "/audio.mp4");
        assert!(parsed.catalog.audio_tracks[0].default);
    }

    #[test]
    fn protected_dynamic_and_unbounded_manifests_fail_explicitly() {
        let base = Url::parse("https://example.test/m.mpd").unwrap();
        assert!(matches!(
            parse(b"<MPD type=\"dynamic\"/>", &base),
            Err(Error::DashDynamicUnsupported)
        ));
        assert!(matches!(
            parse(mpd("<ContentProtection/>").as_bytes(), &base),
            Err(Error::DashDrmUnsupported)
        ));
        let unbounded = mpd(
            "<AdaptationSet mimeType=\"video/mp4\"><SegmentTemplate initialization=\"i\" media=\"$Time$\"><SegmentTimeline><S d=\"1\" r=\"999999999\"/></SegmentTimeline></SegmentTemplate><Representation/></AdaptationSet>",
        );
        assert!(parse(unbounded.as_bytes(), &base).is_err());
        let dtd = "<!DOCTYPE MPD [<!ENTITY secret SYSTEM 'file:///private'>]><MPD>&secret;</MPD>";
        assert!(parse(dtd.as_bytes(), &base).is_err());
    }

    #[test]
    fn audio_bitrates_share_a_group_and_only_preferred_rendition_is_default() {
        let document = mpd(
            "<AdaptationSet mimeType=\"audio/mp4\" lang=\"en\"><SegmentTemplate duration=\"1\" initialization=\"init-$RepresentationID$\" media=\"$RepresentationID$-$Number$\"/><Representation id=\"en-low\" bandwidth=\"32000\"/><Representation id=\"en-high\" bandwidth=\"64000\"/></AdaptationSet><AdaptationSet mimeType=\"audio/mp4\" lang=\"ja\"><SegmentTemplate duration=\"1\" initialization=\"init-$RepresentationID$\" media=\"$RepresentationID$-$Number$\"/><Representation id=\"ja\" bandwidth=\"64000\"/></AdaptationSet>",
        );
        let parsed = parse(
            document.as_bytes(),
            &Url::parse("https://example.test/m.mpd").unwrap(),
        )
        .unwrap();
        let audio = parsed.catalog.audio_tracks;
        assert_eq!(audio.len(), 3);
        assert_eq!(audio[0].group, audio[1].group);
        assert_ne!(audio[1].group, audio[2].group);
        assert_eq!(
            audio
                .iter()
                .filter(|track| track.default)
                .map(|track| track.id.as_str())
                .collect::<Vec<_>>(),
            vec!["en-high"]
        );
    }
}
