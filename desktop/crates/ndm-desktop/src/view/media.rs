use super::*;
use gpui_kit::component::Disableable;

const MAX_AUDIO_TRACKS: usize = 8;

fn toggle_dash_audio(
    selection: &[usize],
    tracks: &[ndm_core::DashAudio],
    index: usize,
) -> Vec<usize> {
    let mut selected = if selection.is_empty() {
        tracks
            .iter()
            .filter(|track| track.default)
            .map(|track| track.index)
            .collect()
    } else {
        selection.to_vec()
    };
    let Some(track) = tracks.iter().find(|track| track.index == index) else {
        return selected;
    };
    if selected.contains(&index) {
        selected.retain(|item| *item != index);
    } else {
        selected.retain(|item| {
            tracks
                .iter()
                .find(|track| track.index == *item)
                .is_some_and(|other| other.group != track.group)
        });
        if selected.len() < MAX_AUDIO_TRACKS {
            selected.push(index);
        }
    }
    selected
}

impl Desktop {
    pub(super) fn inspect_media(&mut self, cx: &mut Context<Self>) {
        if matches!(self.update_state, UpdateState::Installing) || self.inspecting_hls {
            return;
        }
        let url = self.url_input.read(cx).value().trim().to_owned();
        let Ok(parsed) = reqwest::Url::parse(&url) else {
            self.set_notice(
                self.text("请输入有效的媒体链接。", "Enter a valid media URL.")
                    .into(),
                cx,
            );
            cx.notify();
            return;
        };
        if !matches!(parsed.scheme(), "http" | "https") {
            self.set_notice(
                self.text(
                    "媒体链接须使用 HTTP 或 HTTPS。",
                    "Media URLs must use HTTP or HTTPS.",
                )
                .into(),
                cx,
            );
            cx.notify();
            return;
        }
        let dash_first = parsed.path().to_ascii_lowercase().ends_with(".mpd");
        let request = NewTask {
            url: url.clone(),
            destination: self.preferences.download_dir.clone(),
            suggested_filename: None,
            source_url: None,
            expected_size: None,
            headers: BTreeMap::new(),
        };
        let mode = if self.task_route == ProxyMode::Inherit {
            self.preferences.download_route
        } else {
            self.task_route
        };
        let egress = self.preferences.tun_egress.clone();
        let proxy = self.preferences.default_proxy();
        let user_agent = self.preferences.user_agent.clone();
        self.inspecting_hls = true;
        self.clear_notice();
        let result = self.runtime.spawn(async move {
            let cancel = tokio_util::sync::CancellationToken::new();
            let dash = DashOptions {
                proxy: proxy.clone(),
                proxy_mode: mode,
                tun_egress: egress.clone(),
                user_agent: user_agent.clone(),
                ..Default::default()
            };
            if dash_first {
                return inspect_dash_with_options(&request, &dash, cancel)
                    .await
                    .map(MediaCatalog::Dash);
            }
            let hls = HlsOptions {
                proxy,
                proxy_mode: mode,
                tun_egress: egress,
                user_agent,
                ..Default::default()
            };
            match inspect_hls_with_options(&request, &hls, cancel.clone()).await {
                Ok(catalog) => Ok(MediaCatalog::Hls(catalog)),
                Err(_) => inspect_dash_with_options(&request, &dash, cancel)
                    .await
                    .map(MediaCatalog::Dash),
            }
        });
        cx.spawn(async move |view, cx| {
            let result = result.await;
            let _ = view.update(cx, |this, cx| {
                this.inspecting_hls = false;
                if this.url_input.read(cx).value().trim() == url {
                    this.hls_catalog = None;
                    this.dash_catalog = None;
                    this.hls_variant = None;
                    this.hls_audio.clear();
                    this.dash_variant = None;
                    this.dash_audio.clear();
                    match result {
                        Ok(Ok(MediaCatalog::Hls(catalog))) => this.hls_catalog = Some((url, catalog)),
                        Ok(Ok(MediaCatalog::Dash(catalog))) => this.dash_catalog = Some((url, catalog)),
                        Ok(Err(ndm_core::Error::DashDrmUnsupported)) => this.set_notice(this.text(
                            "此 DASH 媒体受 DRM 保护，无法下载。", "This DASH media is DRM-protected and cannot be downloaded.").into(), cx),
                        Ok(Err(ndm_core::Error::DashDynamicUnsupported)) => this.set_notice(this.text(
                            "此链接是动态 DASH 直播，目前支持静态 DASH 和 HLS 直播。", "This is a dynamic DASH stream; static DASH and HLS live streams are supported.").into(), cx),
                        _ => this.set_notice(this.text(
                            "无法读取媒体选项，请检查格式、链接和代理。", "Could not inspect media; check the format, URL and proxy.").into(), cx),
                    }
                }
                cx.notify();
            });
        }).detach();
        cx.notify();
    }

    pub(super) fn dash_options_panel(
        &self,
        catalog: &DashCatalog,
        dark: bool,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let selected = self.dash_variant.or_else(|| {
            catalog
                .representations
                .iter()
                .max_by_key(|item| item.bandwidth)
                .map(|item| item.index)
        });
        let mut qualities = div().flex().flex_wrap().gap_2();
        for representation in &catalog.representations {
            let index = representation.index;
            let resolution = representation
                .resolution
                .map(|(w, h)| format!("{w} × {h}"))
                .unwrap_or_else(|| representation.id.clone());
            qualities = qualities.child(
                Button::new(("dash-representation", index))
                    .small()
                    .when(selected == Some(index), |button| button.primary())
                    .label(format!(
                        "{resolution} · {:.1} Mbps",
                        representation.bandwidth as f64 / 1_000_000.
                    ))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.dash_variant = Some(index);
                        cx.notify();
                    })),
            );
        }
        let mut audio = div().flex().flex_wrap().gap_2().child(
            Button::new("dash-audio-auto")
                .small()
                .when(self.dash_audio.is_empty(), |button| button.primary())
                .label(self.text("自动", "Auto"))
                .on_click(cx.listener(|this, _, _, cx| {
                    this.dash_audio.clear();
                    cx.notify();
                })),
        );
        let tracks = Arc::new(catalog.audio_tracks.clone());
        let effective_audio: Vec<_> = catalog
            .audio_tracks
            .iter()
            .filter(|track| {
                if self.dash_audio.is_empty() {
                    track.default
                } else {
                    self.dash_audio.contains(&track.index)
                }
            })
            .collect();
        for track in &catalog.audio_tracks {
            let index = track.index;
            let chosen = if self.dash_audio.is_empty() {
                track.default
            } else {
                self.dash_audio.contains(&index)
            };
            let label = track
                .label
                .clone()
                .or_else(|| track.language.clone())
                .unwrap_or_else(|| track.id.clone());
            let tracks = tracks.clone();
            audio = audio.child(
                Button::new(("dash-audio", index))
                    .small()
                    .disabled(
                        effective_audio.len() >= MAX_AUDIO_TRACKS
                            && !effective_audio
                                .iter()
                                .any(|selected| selected.group == track.group),
                    )
                    .when(chosen, |button| button.primary())
                    .label(format!("{label} · {} kbps", track.bandwidth / 1000))
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.dash_audio = toggle_dash_audio(&this.dash_audio, &tracks, index);
                        cx.notify();
                    })),
            );
        }
        div()
            .id("dash-options")
            .max_h(px(150.))
            .overflow_y_scroll()
            .rounded_lg()
            .border_1()
            .border_color(rgb(if dark { 0x28344b } else { 0xe3e8f2 }))
            .bg(rgb(if dark { 0x141d2d } else { 0xffffff }))
            .p_3()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(self.text("DASH 清晰度与音轨", "DASH quality and audio")),
            )
            .child(qualities)
            .child(audio)
            .child(
                div()
                    .text_xs()
                    .text_color(rgb(if dark { 0xa0b0cc } else { 0x6c7b94 }))
                    .child(self.text(
                        "音轨最多选 8 个，同一音轨选择一种码率。",
                        "Choose up to 8 tracks, with one bitrate per track.",
                    )),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::toggle_dash_audio;
    fn track(index: usize, group: usize, default: bool) -> ndm_core::DashAudio {
        ndm_core::DashAudio {
            index,
            group,
            default,
            id: index.to_string(),
            language: None,
            label: None,
            bandwidth: 128_000,
            codecs: None,
        }
    }

    #[test]
    fn dash_audio_adds_languages_to_the_default_but_replaces_bitrates_in_a_group() {
        let tracks = vec![track(0, 1, true), track(1, 1, false), track(2, 2, false)];
        let selected = toggle_dash_audio(&[], &tracks, 2);
        assert_eq!(selected, vec![0, 2]);
        let selected = toggle_dash_audio(&selected, &tracks, 1);
        assert_eq!(selected, vec![2, 1]);
        assert_eq!(toggle_dash_audio(&selected, &tracks, 2), vec![1]);
        assert_eq!(toggle_dash_audio(&[1], &tracks, 1), Vec::<usize>::new());
    }

    #[test]
    fn dash_audio_limit_keeps_eight_groups_but_allows_a_bitrate_replacement() {
        let mut tracks: Vec<_> = (0..9)
            .map(|index| track(index, index, index == 0))
            .collect();
        tracks.push(track(9, 0, false));
        let selected: Vec<_> = (0..8).collect();
        assert_eq!(toggle_dash_audio(&selected, &tracks, 8), selected);
        let replaced = toggle_dash_audio(&selected, &tracks, 9);
        assert_eq!(replaced.len(), 8);
        assert!(replaced.contains(&9));
        assert!(!replaced.contains(&0));
    }
}
