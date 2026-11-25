use crate::preferences::ColorScheme;
use gpui_kit::{
    App, Global,
    component::{Theme, theme::ThemeColor},
    rgb,
};

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
}

impl Global for ThemeTone {}
impl From<bool> for ThemeTone {
    fn from(dark: bool) -> Self {
        Self::new(dark, ColorScheme::Classic, [0; 2])
    }
}

impl ThemeTone {
    pub fn new(dark: bool, scheme: ColorScheme, custom: [u32; 2]) -> Self {
        Self {
            darkness: if dark { 1. } else { 0. },
            strength: if scheme == ColorScheme::Classic {
                0.
            } else {
                1.
            },
            accents: scheme.accents(custom),
        }
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
        }
    }

    // Artwork uses violet for primary surfaces and cyan/blue for secondary ones.
    // Map each tint inside its original channel range to retain light/dark contrast.
    // Warm warning, red error and green completion colors keep their meaning.
    fn tint(self, color: u32) -> u32 {
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

    pub fn rgb(self, dark: u32, light: u32) -> u32 {
        mix_rgb(self.tint(light), self.tint(dark), self.darkness)
    }

    pub fn rgba(self, dark: u32, light: u32) -> u32 {
        let alpha = mix_rgb(light & 255, dark & 255, self.darkness);
        (self.rgb(dark >> 8, light >> 8) << 8) | alpha
    }

    pub fn switch_thumb(self, track: u32) -> u32 {
        mix_rgb(0xffffff, contrasting_ink(track), self.strength)
    }

    pub fn style_controls(self, colors: &mut ThemeColor) {
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

fn contrasting_ink(background: u32) -> u32 {
    let linear = |shift: u32| {
        let value = ((background >> shift) & 255) as f32 / 255.;
        if value <= 0.04045 {
            value / 12.92
        } else {
            ((value + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * linear(16) + 0.7152 * linear(8) + 0.0722 * linear(0);
    if luminance > 0.179 { 0 } else { 0xffffff }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silver_neutralizes_surfaces_and_preserves_semantic_colors_and_alpha() {
        for dark in [true, false] {
            let tone = ThemeTone::new(dark, ColorScheme::Silver, [0; 2]);
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
        let custom = ThemeTone::new(true, ColorScheme::Custom, [0xff0000, 0x00ff00]);
        let silver = ThemeTone::new(true, ColorScheme::Silver, [0; 2]);
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
