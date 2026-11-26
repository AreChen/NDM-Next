use super::{CLASSIC_ACCENTS, ThemeTone, luminance, mix_rgb};
use crate::language::Language;
use gpui_kit::{Background, linear_color_stop, linear_gradient, rgb};
use ndm_core::TaskState;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DownloadStatus {
    Completed,
    Failed,
    Paused,
    Downloading,
    Queued,
}

impl DownloadStatus {
    pub const ALL: [Self; 5] = [
        Self::Completed,
        Self::Failed,
        Self::Paused,
        Self::Downloading,
        Self::Queued,
    ];

    pub fn label(self, language: Language) -> &'static str {
        match self {
            Self::Completed => language.text("已完成", "Completed"),
            Self::Failed => language.text("失败", "Failed"),
            Self::Paused => language.text("已暂停", "Paused"),
            Self::Downloading => language.text("下载中", "Downloading"),
            Self::Queued => language.text("等待中", "Queued"),
        }
    }

    pub fn default_color(self) -> u32 {
        match self {
            Self::Completed => 0x3ed4b2,
            Self::Failed => 0xe77b8e,
            Self::Paused => 0xe7ba6f,
            Self::Downloading => 0x885dff,
            Self::Queued => 0x8396b9,
        }
    }
}

impl From<TaskState> for DownloadStatus {
    fn from(state: TaskState) -> Self {
        match state {
            TaskState::Completed => Self::Completed,
            TaskState::Failed => Self::Failed,
            TaskState::Paused => Self::Paused,
            TaskState::Running => Self::Downloading,
            TaskState::Prepared | TaskState::Queued => Self::Queued,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub(crate) struct DownloadColors {
    completed: Option<u32>,
    failed: Option<u32>,
    paused: Option<u32>,
    downloading: Option<u32>,
    queued: Option<u32>,
}

impl DownloadColors {
    pub fn get(self, status: DownloadStatus) -> Option<u32> {
        match status {
            DownloadStatus::Completed => self.completed,
            DownloadStatus::Failed => self.failed,
            DownloadStatus::Paused => self.paused,
            DownloadStatus::Downloading => self.downloading,
            DownloadStatus::Queued => self.queued,
        }
    }

    pub fn set(&mut self, status: DownloadStatus, color: Option<u32>) {
        let entry = match status {
            DownloadStatus::Completed => &mut self.completed,
            DownloadStatus::Failed => &mut self.failed,
            DownloadStatus::Paused => &mut self.paused,
            DownloadStatus::Downloading => &mut self.downloading,
            DownloadStatus::Queued => &mut self.queued,
        };
        *entry = color.map(|value| value.min(0xffffff));
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct DownloadStyle {
    pub background: [u32; 2],
    pub edge: u32,
    pub accent: u32,
    pub progress: [u32; 2],
}

impl DownloadStyle {
    pub fn row_background(self) -> Background {
        linear_gradient(
            105.,
            linear_color_stop(rgb(self.background[0]), 0.),
            linear_color_stop(rgb(self.background[1]), 1.),
        )
    }

    pub fn progress_background(self) -> Background {
        linear_gradient(
            90.,
            linear_color_stop(rgb(self.progress[0]), 0.),
            linear_color_stop(rgb(self.progress[1]), 1.),
        )
    }

    pub fn new(tone: &ThemeTone, status: DownloadStatus, color: Option<u32>) -> Self {
        if let Some(color) = color {
            return Self::custom(tone, color);
        }
        let (dark, light) = match status {
            DownloadStatus::Downloading => (
                [0x252044, 0x183e69, 0x49418c, 0xb19bff],
                [0xf0ebff, 0xdeefff, 0xc6c0f2, 0x6e4dcc],
            ),
            DownloadStatus::Completed => (
                [0x172238, 0x124b40, 0x286055, 0x60e1bc],
                [0xf3f8fc, 0xdaf4e9, 0xb4dccc, 0x168262],
            ),
            DownloadStatus::Paused => (
                [0x222435, 0x443424, 0x62513c, 0xe7ba6f],
                [0xf8f7fc, 0xffefd7, 0xe7d3af, 0x9a651d],
            ),
            DownloadStatus::Failed => (
                [0x252237, 0x4c2836, 0x764052, 0xff929d],
                [0xf8f4fc, 0xffe2e8, 0xeabdc7, 0xb93752],
            ),
            DownloadStatus::Queued => (
                [0x1a2134, 0x1c2b42, 0x303e59, 0x9facca],
                [0xffffff, 0xedf3fc, 0xdce4f1, 0x697c9c],
            ),
        };
        let [start, end, edge, accent] =
            std::array::from_fn(|i| mix_rgb(light[i], dark[i], tone.darkness));
        Self {
            background: [start, end],
            edge,
            accent,
            progress: [
                CLASSIC_ACCENTS[0],
                match status {
                    DownloadStatus::Completed => 0x3ed4b2,
                    DownloadStatus::Failed | DownloadStatus::Paused => accent,
                    _ => CLASSIC_ACCENTS[1],
                },
            ],
        }
    }

    fn custom(tone: &ThemeTone, color: u32) -> Self {
        let dark = tone.darkness >= 0.5;
        let base = tone.surface(0x192337, 0xf8f9fd);
        let background = [
            mix_rgb(base, color, 0.06),
            mix_rgb(base, color, if dark { 0.21 } else { 0.12 }),
        ];
        let accent = status_ink(color, tone, background);
        Self {
            background,
            edge: mix_rgb(background[1], accent, 0.27),
            accent,
            progress: [
                mix_rgb(accent, if dark { 0xffffff } else { 0 }, 0.18),
                accent,
            ],
        }
    }

    pub(super) fn blend(self, to: Self, t: f32) -> Self {
        Self {
            background: std::array::from_fn(|i| mix_rgb(self.background[i], to.background[i], t)),
            edge: mix_rgb(self.edge, to.edge, t),
            accent: mix_rgb(self.accent, to.accent, t),
            progress: std::array::from_fn(|i| mix_rgb(self.progress[i], to.progress[i], t)),
        }
    }
}

/// Keep small status text legible over the row, its pill, and the detail window.
fn status_ink(color: u32, tone: &ThemeTone, row: [u32; 2]) -> u32 {
    let dark = tone.darkness >= 0.5;
    let badge_opacity = if dark { 13. / 255. } else { 128. / 255. };
    let backgrounds = row
        .into_iter()
        .chain(row.map(|color| mix_rgb(color, 0xffffff, badge_opacity)))
        .chain(tone.floating_background());
    let neutral = if dark { 0xffffff } else { 0 };
    // Test the highest-luminance background in dark mode and the lowest in light mode.
    let backdrop = backgrounds.map(luminance).fold(
        if dark { 0. } else { 1. },
        if dark { f32::max } else { f32::min },
    );
    let legible = |color| {
        let value = luminance(color);
        if dark {
            (value + 0.05) / (backdrop + 0.05) >= 4.55
        } else {
            (backdrop + 0.05) / (value + 0.05) >= 4.55
        }
    };
    if legible(color) {
        return color;
    }
    let (mut low, mut high) = (0, 255);
    while high - low > 1 {
        let middle = (low + high) / 2;
        if legible(mix_rgb(color, neutral, middle as f32 / 255.)) {
            high = middle;
        } else {
            low = middle;
        }
    }
    mix_rgb(color, neutral, high as f32 / 255.)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{palette::CustomPalette, preferences::ColorScheme};

    #[test]
    fn download_states_keep_their_default_colors_in_every_preset() {
        for dark in [false, true] {
            let tone = |scheme| {
                ThemeTone::new(
                    dark,
                    scheme,
                    CustomPalette {
                        primary: 0x777777,
                        secondary: Some(0xaaaaaa),
                        ..CustomPalette::default()
                    },
                    DownloadColors::default(),
                )
            };
            let classic = tone(ColorScheme::Classic);
            for scheme in ColorScheme::ALL {
                for status in DownloadStatus::ALL {
                    assert_eq!(tone(scheme).download(status), classic.download(status));
                }
            }
            let running = tone(ColorScheme::Silver).download(TaskState::Running.into());
            assert_eq!(running.progress, CLASSIC_ACCENTS);
            assert_eq!(running.accent, if dark { 0xb19bff } else { 0x6e4dcc });
        }
    }

    #[test]
    fn overrides_affect_only_the_selected_state_and_follow_theme_transitions() {
        let palette = CustomPalette::default();
        for scheme in [
            ColorScheme::Classic,
            ColorScheme::Silver,
            ColorScheme::Custom,
        ] {
            let default = ThemeTone::new(false, scheme, palette, DownloadColors::default());
            let mut colors = DownloadColors::default();
            colors.set(DownloadStatus::Failed, Some(0x2f66ec));
            let custom = ThemeTone::new(false, scheme, palette, colors);
            let other = default.blend(custom, 0.45);
            for status in DownloadStatus::ALL {
                let initial = default.download(status);
                let changed = custom.download(status);
                if status == DownloadStatus::Failed {
                    assert_ne!(changed.accent, initial.accent);
                    assert_ne!(changed.background, initial.background);
                    assert_ne!(changed.progress, initial.progress);
                    assert_ne!(other.download(status), initial);
                    assert_ne!(other.download(status), changed);
                } else {
                    assert_eq!(changed, initial);
                    assert_eq!(other.download(status), initial);
                }
            }
            colors.set(DownloadStatus::Failed, None);
            assert_eq!(ThemeTone::new(false, scheme, palette, colors), default);
        }
        assert_eq!(
            DownloadStatus::from(TaskState::Prepared),
            DownloadStatus::Queued
        );
        assert_eq!(
            DownloadStatus::from(TaskState::Paused),
            DownloadStatus::Paused
        );
    }

    #[test]
    fn extreme_custom_colors_keep_status_text_readable_in_both_modes() {
        let seeds = [
            0x000000, 0xffffff, 0x777777, 0xff0000, 0x00ff00, 0x0000ff, 0xffff00,
        ];
        for dark in [true, false] {
            for surface in [0x000000, 0xffffff, 0xe01d90] {
                let tone = ThemeTone::new(
                    dark,
                    ColorScheme::Custom,
                    CustomPalette {
                        surface: Some(surface),
                        ..CustomPalette::default()
                    },
                    DownloadColors::default(),
                );
                for seed in seeds {
                    let style = DownloadStyle::new(&tone, DownloadStatus::Completed, Some(seed));
                    let pill_alpha = if dark { 13. / 255. } else { 128. / 255. };
                    for background in style
                        .background
                        .into_iter()
                        .chain(style.background.map(|bg| mix_rgb(bg, 0xffffff, pill_alpha)))
                        .chain(tone.floating_background())
                    {
                        let text = luminance(style.accent);
                        let bg = luminance(background);
                        let ratio = (text.max(bg) + 0.05) / (text.min(bg) + 0.05);
                        assert!(
                            ratio >= 4.5,
                            "color {seed:#06x} on {background:#06x}: {ratio}"
                        );
                    }
                }
            }
        }
    }
}
