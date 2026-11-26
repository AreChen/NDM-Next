use crate::preferences::ColorScheme;
use gpui_kit::{
    App, Global, Hsla, Rgba,
    component::{Theme, theme::ThemeColor},
    hsla, rgb,
};
use serde::{Deserialize, Serialize};

mod downloads;
pub(crate) use downloads::{DownloadColors, DownloadStatus, DownloadStyle};

pub(crate) const CLASSIC_ACCENTS: [u32; 2] = [0x885dff, 0x37c9f5];

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct CustomPalette {
    pub primary: u32,
    pub secondary: Option<u32>,
    pub surface: Option<u32>,
    // The pre-0.8.1 palette tinted surfaces using the two accents.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub legacy_surfaces: bool,
}

impl Default for CustomPalette {
    fn default() -> Self {
        Self {
            primary: CLASSIC_ACCENTS[0],
            secondary: None,
            surface: None,
            legacy_surfaces: false,
        }
    }
}

impl CustomPalette {
    pub fn accents(self) -> [u32; 2] {
        let base: Hsla = rgb(self.primary).into();
        let secondary = self.secondary.unwrap_or_else(|| {
            // An analogous hue, with a lighter value to distinguish progress.
            rgb_value(hsla(
                (base.h - 0.15).rem_euclid(1.),
                base.s * 0.82,
                0.47 + 0.22 * base.l,
                1.,
            ))
        });
        [self.primary, secondary]
    }

    pub fn surface(self) -> u32 {
        self.surface
            .unwrap_or_else(|| mix_rgb(0x778090, self.primary, 0.55))
    }
}

pub(crate) fn rgb_value(color: Hsla) -> u32 {
    let Rgba { r, g, b, .. } = color.into();
    let byte = |value: f32| (value * 255.).round() as u32;
    (byte(r) << 16) | (byte(g) << 8) | byte(b)
}

pub(crate) fn mix_rgb(from: u32, to: u32, t: f32) -> u32 {
    if t <= 0. {
        return from;
    }
    if t >= 1. {
        return to;
    }
    let channel = |shift| {
        let a = ((from >> shift) & 255u32) as f32;
        let b = ((to >> shift) & 255u32) as f32;
        (a + (b - a) * t).round() as u32
    };
    channel(16u32) << 16 | channel(8u32) << 8 | channel(0u32)
}

/// All windows read the same presented colors, including mid-transition.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct ThemeTone {
    pub darkness: f32,
    strength: f32,
    accents: [u32; 2],
    surface_color: u32,
    surface_strength: f32,
    downloads: [DownloadStyle; DownloadStatus::ALL.len()],
}

impl Global for ThemeTone {}
impl From<bool> for ThemeTone {
    fn from(dark: bool) -> Self {
        Self::new(
            dark,
            ColorScheme::Classic,
            CustomPalette::default(),
            DownloadColors::default(),
        )
    }
}

impl ThemeTone {
    pub fn new(
        dark: bool,
        scheme: ColorScheme,
        custom: CustomPalette,
        colors: DownloadColors,
    ) -> Self {
        let mut tone = Self {
            darkness: if dark { 1. } else { 0. },
            strength: if scheme == ColorScheme::Classic {
                0.
            } else {
                1.
            },
            accents: scheme.accents(custom),
            surface_color: custom.surface(),
            surface_strength: f32::from(scheme == ColorScheme::Custom && !custom.legacy_surfaces),
            downloads: [DownloadStyle::default(); DownloadStatus::ALL.len()],
        };
        for status in DownloadStatus::ALL {
            tone.downloads[status as usize] = DownloadStyle::new(&tone, status, colors.get(status));
        }
        tone
    }

    pub fn current(cx: &App) -> Self {
        cx.try_global::<Self>()
            .copied()
            .unwrap_or_else(|| Theme::global(cx).mode.is_dark().into())
    }

    pub fn blend(self, to: Self, t: f32) -> Self {
        if t >= 1. {
            return to;
        }
        Self {
            darkness: self.darkness + (to.darkness - self.darkness) * t,
            strength: self.strength + (to.strength - self.strength) * t,
            accents: std::array::from_fn(|i| mix_rgb(self.accents[i], to.accents[i], t)),
            surface_color: mix_rgb(self.surface_color, to.surface_color, t),
            surface_strength: self.surface_strength
                + (to.surface_strength - self.surface_strength) * t,
            downloads: std::array::from_fn(|i| self.downloads[i].blend(to.downloads[i], t)),
        }
    }

    pub fn download(&self, status: DownloadStatus) -> DownloadStyle {
        self.downloads[status as usize]
    }

    pub fn floating_background(&self) -> [u32; 2] {
        [
            self.surface(0x151a2e, 0xf8f7ff),
            self.surface(0x101d27, 0xf2f8fb),
        ]
    }

    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    pub fn browser_appearance(&self) -> ndm_host::appearance::Appearance {
        use ndm_host::appearance::{Appearance, Mode};

        Appearance {
            mode: if self.darkness == 1. {
                Mode::Dark
            } else {
                Mode::Light
            },
            primary: self.accents[0],
            secondary: self.accents[1],
            surface: self.surface(0x151b2c, 0xfcfdff),
        }
    }

    // Artwork uses violet for primary surfaces and cyan/blue for secondary ones.
    // Map each tint inside its original channel range to retain light/dark contrast.
    // Warm warning, red error and green completion colors keep their meaning.
    fn tint(&self, color: u32) -> u32 {
        let (r, g, b) = (color >> 16, (color >> 8) & 255, color & 255);
        if self.strength == 0. || b < r || b < g || r == g && g == b {
            return color;
        }
        let accent = self.accents[usize::from(g > r)];
        let low = r.min(g);
        let chroma = b - low;
        let channel = |shift: u32| low + (chroma * ((accent >> shift) & 255) + 127) / 255;
        let tinted = channel(16) << 16 | channel(8) << 8 | channel(0);
        mix_rgb(color, tinted, self.strength)
    }

    pub fn rgb(&self, dark: u32, light: u32) -> u32 {
        mix_rgb(self.tint(light), self.tint(dark), self.darkness)
    }

    /// Preserve the separation of surfaces and the readability of their text in either mode.
    pub fn surface(&self, dark: u32, light: u32) -> u32 {
        let dark_tint = mix_rgb(dark, self.surface_color, 0.16);
        let light_tint = mix_rgb(light, self.surface_color, 0.12);
        mix_rgb(
            self.rgb(dark, light),
            mix_rgb(light_tint, dark_tint, self.darkness),
            self.surface_strength,
        )
    }

    pub fn rgba(&self, dark: u32, light: u32) -> u32 {
        let alpha = mix_rgb(light & 255, dark & 255, self.darkness);
        (self.rgb(dark >> 8, light >> 8) << 8) | alpha
    }

    pub fn switch_thumb(&self, track: u32) -> u32 {
        mix_rgb(0xffffff, contrasting_ink(track), self.strength)
    }

    pub fn style_controls(&self, colors: &mut ThemeColor) {
        let accent = self.rgb(0xa488ff, 0x6544b4);
        let fill = rgb(accent).into();
        let ink = rgb(contrasting_ink(accent)).into();
        colors.primary = fill;
        colors.button_primary = fill;
        colors.primary_foreground = ink;
        colors.button_primary_foreground = ink;
        colors.primary_hover = rgb(mix_rgb(accent, contrasting_ink(accent), 0.08)).into();
        colors.button_primary_hover = colors.primary_hover;
        colors.primary_active = rgb(mix_rgb(accent, contrasting_ink(accent), 0.14)).into();
        colors.button_primary_active = colors.primary_active;
        colors.ring = fill;
        colors.caret = fill;
        colors.link = fill;
        colors.link_hover = colors.primary_hover;
        colors.link_active = colors.primary_active;
        colors.progress_bar = fill;
        colors.list_active_border = fill;
        colors.selection = rgb(accent).opacity(0.3).into();
    }
}

fn luminance(color: u32) -> f32 {
    let linear = |shift: u32| {
        let value = ((color >> shift) & 255) as f32 / 255.;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * linear(16) + 0.7152 * linear(8) + 0.0722 * linear(0)
}

fn contrasting_ink(background: u32) -> u32 {
    if luminance(background) > 0.179 {
        0
    } else {
        0xffffff
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(any(windows, target_os = "macos", target_os = "linux"))]
    #[test]
    fn browser_appearance_uses_the_effective_mode_and_selected_palette() {
        use ndm_host::appearance::Mode;

        let custom = CustomPalette {
            primary: 0x912620,
            secondary: Some(0x12bdee),
            surface: Some(0xe0b120),
            ..CustomPalette::default()
        };
        for (dark, mode) in [(true, Mode::Dark), (false, Mode::Light)] {
            for scheme in ColorScheme::ALL {
                let tone = ThemeTone::new(dark, scheme, custom, DownloadColors::default());
                let appearance = tone.browser_appearance();
                let [primary, secondary] = scheme.accents(custom);
                assert_eq!(appearance.mode, mode);
                assert_eq!(
                    (appearance.primary, appearance.secondary),
                    (primary, secondary)
                );
                assert_eq!(appearance.surface, tone.surface(0x151b2c, 0xfcfdff));
            }
        }
        let automatic = ThemeTone::new(
            false,
            ColorScheme::Custom,
            CustomPalette::default(),
            DownloadColors::default(),
        );
        let changed = ThemeTone::new(
            false,
            ColorScheme::Custom,
            custom,
            DownloadColors::default(),
        );
        assert_ne!(
            automatic.browser_appearance().surface,
            changed.browser_appearance().surface
        );
    }

    #[test]
    fn silver_neutralizes_surfaces_and_preserves_semantic_colors_and_alpha() {
        for dark in [true, false] {
            let tone = ThemeTone::new(
                dark,
                ColorScheme::Silver,
                CustomPalette::default(),
                DownloadColors::default(),
            );
            for value in [
                tone.rgb(0x0c1120, 0xf4f6fb),
                tone.rgb(0xb19bff, 0x6e4dcc),
                tone.rgb(0x37c9f5, 0x37c9f5),
            ] {
                assert_eq!(value >> 16, value & 255);
                assert_eq!((value >> 8) & 255, value & 255);
            }
            assert_eq!(tone.rgb(0xb93752, 0xb93752), 0xb93752);
            assert_eq!(
                tone.rgba(0x885dff55, 0x885dff20) & 255,
                if dark { 0x55 } else { 0x20 }
            );
        }
    }

    #[test]
    fn custom_primary_and_secondary_are_independent_and_retarget_without_a_jump() {
        let custom = ThemeTone::new(
            true,
            ColorScheme::Custom,
            CustomPalette {
                primary: 0xff0000,
                secondary: Some(0x00ff00),
                ..CustomPalette::default()
            },
            DownloadColors::default(),
        );
        let silver = ThemeTone::new(
            true,
            ColorScheme::Silver,
            CustomPalette::default(),
            DownloadColors::default(),
        );
        assert_eq!(custom.rgb(0xaa00ff, 0), 0xff0000);
        assert_eq!(custom.rgb(0x00aaff, 0), 0x00ff00);
        let halfway = silver.blend(custom, 0.5);
        assert_ne!(halfway, silver);
        assert_eq!(halfway.blend(silver, 0.), halfway);
        assert_eq!(halfway.blend(silver, 1.), silver);
        assert_eq!(contrasting_ink(0xffffff), 0);
        assert_eq!(contrasting_ink(0), 0xffffff);
    }
}
