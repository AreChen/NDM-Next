use gpui_kit::{
    App, ElementId, Global, Hsla, Pixels, Rgba, SharedString, Window,
    component::{Theme as ComponentTheme, theme::ThemeColor},
};
use gpui_motion_kit::core::{Easing, Transition};
use std::{rc::Rc, sync::Arc, time::Duration};

/// Presentation state is shared by all windows, independently of target mode.
#[derive(Clone, Copy)]
pub(crate) struct ThemeTone(pub f32);
impl Global for ThemeTone {}
impl From<bool> for ThemeTone {
    fn from(dark: bool) -> Self {
        Self(if dark { 1. } else { 0. })
    }
}
impl ThemeTone {
    pub fn current(cx: &App) -> Self {
        cx.try_global::<Self>()
            .copied()
            .unwrap_or_else(|| ComponentTheme::global(cx).mode.is_dark().into())
    }
    pub fn rgb(self, dark: u32, light: u32) -> u32 {
        let channel = |shift: u32| {
            let a = ((light >> shift) & 255u32) as f32;
            let b = ((dark >> shift) & 255u32) as f32;
            (a + (b - a) * self.0.clamp(0., 1.)).round() as u32
        };
        (channel(16) << 16) | (channel(8) << 8) | channel(0)
    }
    pub fn rgba(self, dark: u32, light: u32) -> u32 {
        let alpha = (light & 255) as f32 + ((dark & 255) as f32 - (light & 255) as f32) * self.0;
        (self.rgb(dark >> 8, light >> 8) << 8) | alpha.round() as u32
    }
}

pub(crate) struct ThemeTransition {
    progress: gpui_motion_kit::ValueState,
    from: ThemeColor,
    to: ThemeColor,
    from_tone: ThemeTone,
    to_tone: ThemeTone,
}

impl ThemeTransition {
    pub fn begin(mode: crate::preferences::Theme, cx: &mut App) -> Option<Self> {
        let from = ComponentTheme::global(cx).colors;
        let from_tone = ThemeTone::current(cx);
        crate::apply_theme(mode, cx);
        let to = ComponentTheme::global(cx).colors;
        let to_tone = ThemeTone::from(ComponentTheme::global(cx).mode.is_dark());
        if cx.reduce_motion() || (from == to && from_tone.0 == to_tone.0) {
            cx.set_global(to_tone);
            return None;
        }
        // Restore the presented palette before a frame can paint the new mode.
        ComponentTheme::update(cx, |theme| theme.colors = from);
        Some(Self {
            progress: gpui_motion_kit::ValueState::new(0.),
            from,
            to,
            from_tone,
            to_tone,
        })
    }

    pub fn frame(&mut self, window: &mut Window, cx: &mut App) -> bool {
        let frame = self.progress.frame(1., tween(280), window, cx);
        let t = frame.value() as f32;
        let colors = blend_palette(self.from, self.to, t);
        cx.set_global(ThemeTone(
            self.from_tone.0 + (self.to_tone.0 - self.from_tone.0) * t,
        ));
        // Theme::update refreshes every window. Don't feed an unchanged sample
        // back into rendering (especially while the frame clock is stationary).
        if ComponentTheme::global(cx).colors != colors {
            ComponentTheme::update(cx, |theme| theme.colors = colors);
        }
        frame.is_animating()
    }
}

fn blend_color(from: Hsla, to: Hsla, t: f32) -> Hsla {
    let a = Rgba::from(from);
    let b = Rgba::from(to);
    Rgba {
        r: a.r + (b.r - a.r) * t,
        g: a.g + (b.g - a.g) * t,
        b: a.b + (b.b - a.b) * t,
        a: a.a + (b.a - a.a) * t,
    }
    .into()
}

// Kit's solid color fields are projected through Theme::update so semantic
// tokens, inputs, switches and native window controls transition together.
fn blend_palette(from: ThemeColor, to: ThemeColor, t: f32) -> ThemeColor {
    if t <= 0. {
        return from;
    }
    if t >= 1. {
        return to;
    }
    let mut result = to;
    macro_rules! blend { ($($field:ident),* $(,)?) => { $(result.$field = blend_color(from.$field, to.$field, t);)* }; }
    blend!(
        accent,
        accent_foreground,
        accordion,
        background,
        border,
        button,
        button_active,
        button_foreground,
        button_hover,
        button_danger,
        button_danger_active,
        button_danger_foreground,
        button_danger_hover,
        button_info,
        button_info_active,
        button_info_foreground,
        button_info_hover,
        button_primary,
        button_primary_active,
        button_primary_foreground,
        button_primary_hover,
        button_secondary,
        button_secondary_active,
        button_secondary_foreground,
        button_secondary_hover,
        button_success,
        button_success_active,
        button_success_foreground,
        button_success_hover,
        button_warning,
        button_warning_active,
        button_warning_foreground,
        button_warning_hover,
        group_box,
        group_box_foreground,
        caret,
        chart_1,
        chart_2,
        chart_3,
        chart_4,
        chart_5,
        chart_bullish,
        chart_bearish,
        chart_grid,
        danger,
        danger_active,
        danger_foreground,
        danger_hover,
        description_list_label,
        description_list_label_foreground,
        drag_border,
        drop_target,
        foreground,
        info,
        info_active,
        info_foreground,
        info_hover,
        input,
        link,
        link_active,
        link_hover,
        list,
        list_active,
        list_active_border,
        list_even,
        list_head,
        list_hover,
        list_highlight_foreground,
        muted,
        muted_foreground,
        popover,
        popover_foreground,
        primary,
        primary_active,
        primary_foreground,
        primary_hover,
        progress_bar,
        ring,
        scrollbar,
        scrollbar_thumb,
        scrollbar_thumb_hover,
        secondary,
        secondary_active,
        secondary_foreground,
        secondary_hover,
        selection,
        sidebar,
        sidebar_accent,
        sidebar_accent_foreground,
        sidebar_border,
        sidebar_foreground,
        sidebar_primary,
        sidebar_primary_foreground,
        skeleton,
        slider_bar,
        slider_thumb,
        success,
        success_foreground,
        success_hover,
        success_active,
        switch,
        switch_thumb,
        tab,
        tab_active,
        tab_active_foreground,
        tab_bar,
        tab_bar_segmented,
        tab_foreground,
        table,
        table_active,
        table_active_border,
        table_even,
        table_head,
        table_head_foreground,
        table_foot,
        table_foot_foreground,
        table_hover,
        table_row_border,
        title_bar,
        title_bar_border,
        status_bar,
        status_bar_border,
        warning,
        warning_active,
        warning_hover,
        warning_foreground,
        overlay,
        window_border,
        red,
        red_light,
        green,
        green_light,
        blue,
        blue_light,
        yellow,
        yellow_light,
        magenta,
        magenta_light,
        cyan,
        cyan_light
    );
    result
}

/// Scramble only translated UI labels, never filenames, paths, URLs or values.
pub(crate) fn scramble(text: &str, progress: f32) -> String {
    if progress >= 1. {
        return text.to_owned();
    }
    let length = text.chars().count();
    let revealed = (length as f32 * progress.clamp(0., 1.)) as usize;
    let tick = (progress * 10.) as usize;
    const GLYPHS: &[char] = &['#', '+', '×', '/', '·', '≡'];
    text.chars()
        .enumerate()
        .map(|(i, ch)| {
            if i < revealed || ch.is_whitespace() {
                ch
            } else {
                GLYPHS[(i + tick) % GLYPHS.len()]
            }
        })
        .collect()
}

type SegmentChange = dyn Fn(&usize, &mut Window, &mut App);

#[derive(gpui_kit::IntoElement)]
pub(crate) struct SegmentedControl {
    pub id: ElementId,
    /// Stable button IDs and the labels shown in this language.
    pub segments: Vec<(SharedString, SharedString)>,
    pub selected: Option<usize>,
    pub accessibility_label: Option<SharedString>,
    pub width: Option<Pixels>,
    pub disabled: bool,
    pub on_change: Rc<SegmentChange>,
}

impl gpui_kit::RenderOnce for SegmentedControl {
    fn render(self, window: &mut Window, cx: &mut App) -> impl gpui_kit::IntoElement {
        use gpui_kit::{
            component::{
                Disableable, Sizable,
                button::{Button, ButtonCustomVariant, ButtonVariants},
            },
            prelude::*,
            *,
        };
        use gpui_motion_kit::{
            MotionState,
            core::{Motion, Target},
        };

        let tone = ThemeTone::current(cx);
        // The sliding indicator supplies the only background for each segment.
        let button =
            ButtonCustomVariant::new(cx).foreground(rgb(tone.rgb(0xede8ff, 0x44366b)).into());
        let count = self.segments.len() as f32;
        let mut track = div().relative().h(px(24.)).w_full().flex();
        if let Some(index) = self.selected {
            let key = ElementId::NamedChild(Arc::new(self.id.clone()), "position".into());
            let state = window.use_keyed_state(key, cx, |_, _| MotionState::default());
            let position = state.update(cx, |state, cx| {
                state
                    .frame(
                        Motion::new(Target::default().x(index as f32)).transition(tween(180)),
                        window,
                        cx,
                    )
                    .target()
                    .x_px()
            });
            let debug_id = format!("{}-indicator", self.id);
            track = track.child(
                div()
                    .id("segment-indicator")
                    .debug_selector(move || debug_id)
                    .absolute()
                    .top_0()
                    .left(relative(position / count))
                    .w(relative(1. / count))
                    .h_full()
                    .rounded_md()
                    .border_1()
                    .border_color(rgba(0xa78bfa55))
                    .bg(rgb(tone.rgb(0x51447e, 0xe5ddff))),
            );
        }
        for (index, (id, label)) in self.segments.into_iter().enumerate() {
            let on_change = self.on_change.clone();
            let accessible_name = self
                .accessibility_label
                .as_ref()
                .map_or_else(|| label.clone(), |group| format!("{group} {label}").into());
            track = track.child(
                Button::new(id.clone())
                    .custom(button)
                    .small()
                    .disabled(self.disabled)
                    .toggled(self.selected == Some(index))
                    .flex_1()
                    .min_w_0()
                    .h(px(24.))
                    .px_0()
                    .label(label)
                    .accessibility_label(accessible_name)
                    .text_size(px(11.))
                    .debug_selector(move || id.to_string())
                    .on_click(move |_, window, cx| on_change(&index, window, cx)),
            );
        }
        div()
            .id(self.id)
            .when(self.width.is_none(), |group| group.flex_1())
            .when_some(self.width, |group, width| {
                group.w(width).max_w(relative(1.))
            })
            .min_w_0()
            .p(px(2.))
            .rounded_lg()
            .border_1()
            .border_color(rgb(tone.rgb(0x3b4160, 0xd9deec)))
            .bg(rgb(tone.rgb(0x151c2d, 0xf0f2f8)))
            .opacity(if self.disabled { 0.55 } else { 1. })
            .block_mouse_except_scroll()
            .child(track)
    }
}

#[derive(gpui_kit::IntoElement)]
pub(crate) struct Switch {
    id: &'static str,
    checked: bool,
    label: gpui_kit::SharedString,
    tooltip: Option<gpui_kit::SharedString>,
    on_change: Option<std::rc::Rc<SwitchChange>>,
}
type SwitchChange = dyn Fn(&bool, &mut Window, &mut App);

impl Switch {
    pub fn new(id: &'static str) -> Self {
        Self {
            id,
            checked: false,
            label: "".into(),
            tooltip: None,
            on_change: None,
        }
    }
    pub fn small(self) -> Self {
        self
    }
    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }
    pub fn label(mut self, label: impl Into<gpui_kit::SharedString>) -> Self {
        self.label = label.into();
        self
    }
    pub fn tooltip(mut self, text: impl Into<gpui_kit::SharedString>) -> Self {
        self.tooltip = Some(text.into());
        self
    }
    pub fn on_click(mut self, callback: impl Fn(&bool, &mut Window, &mut App) + 'static) -> Self {
        self.on_change = Some(std::rc::Rc::new(callback));
        self
    }
}

impl gpui_kit::RenderOnce for Switch {
    fn render(self, window: &mut Window, cx: &mut App) -> impl gpui_kit::IntoElement {
        use gpui_kit::{component::tooltip::Tooltip, prelude::*, *};
        use gpui_motion_kit::{
            MotionState,
            core::{Motion, Target},
        };
        let state = window.use_keyed_state((self.id, 0usize), cx, |_, _| MotionState::default());
        let progress = state.update(cx, |state, cx| {
            state
                .frame(
                    Motion::new(Target::default().x(if self.checked { 1. } else { 0. }))
                        .transition(tween(180)),
                    window,
                    cx,
                )
                .target()
                .x_px()
        });
        let tone = ThemeTone::current(cx);
        let id = self.id;
        let track = div()
            .id((id, 2usize))
            .debug_selector(move || format!("{id}-track"))
            .relative()
            .w(px(30.))
            .h(px(18.))
            .flex_shrink_0()
            .rounded_full()
            .bg(rgb(
                ThemeTone(progress).rgb(0x8864ed, tone.rgb(0x3c475c, 0xc8d0e0))
            ))
            .child(
                div()
                    .id((id, 3usize))
                    .debug_selector(move || format!("{id}-thumb"))
                    .absolute()
                    .top(px(2.))
                    .left(px(2. + 12. * progress))
                    .size(px(14.))
                    .rounded_full()
                    .bg(rgb(0xffffff)),
            );
        gpui_kit::base::Switch::new(self.id)
            .checked(self.checked)
            .accessibility_label(self.label.clone())
            .flex()
            .items_center()
            .gap_2()
            .h(px(28.))
            .rounded_md()
            .cursor_pointer()
            .border_1()
            .border_color(rgba(0))
            .focus_visible(|style| style.border_color(rgb(0x9b85ed)))
            .on_change(move |next, _, window, cx| {
                if let Some(callback) = &self.on_change {
                    callback(&next, window, cx);
                }
            })
            .child(div().id((self.id, 1usize)).child(track).when_some(
                self.tooltip,
                |track, tooltip| {
                    track.tooltip(move |_, cx| cx.new(|_| Tooltip::new(tooltip.clone())).into())
                },
            ))
            .child(div().text_xs().child(self.label))
    }
}

pub(crate) fn tween(milliseconds: u64) -> Transition {
    Transition::Tween(Duration::from_millis(milliseconds), Easing::EaseOut)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_kit::{Context, Modifiers, Render, TestAppContext, div, prelude::*, px, rgb};
    use gpui_motion_kit::{
        MotionState,
        core::{Motion, Target},
    };

    #[test]
    fn theme_tones_and_scramble_resolve_without_touching_nonlabels() {
        assert_eq!(ThemeTone(0.).rgb(0x112233, 0xaabbcc), 0xaabbcc);
        assert_eq!(ThemeTone(1.).rgb(0x112233, 0xaabbcc), 0x112233);
        assert_eq!(ThemeTone(0.5).rgb(0xffffff, 0), 0x808080);
        for label in ["设置中心", "Settings center", "中文 / English"] {
            assert_eq!(scramble(label, 1.), label);
            assert_eq!(scramble(label, 2.), label);
            assert_eq!(scramble(label, 0.3).chars().count(), label.chars().count());
            assert_ne!(scramble(label, 0.3), label);
        }
    }

    #[gpui_kit::test]
    fn theme_transition_preserves_first_frame_retargets_and_settles(cx: &mut TestAppContext) {
        struct ThemeView(Option<ThemeTransition>);
        impl Render for ThemeView {
            fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                if let Some(transition) = &mut self.0
                    && !transition.frame(window, cx)
                {
                    self.0 = None;
                }
                div()
                    .size_full()
                    .bg(rgb(ThemeTone::current(cx).rgb(0x0c1120, 0xf4f6fb)))
            }
        }
        cx.update(|cx| {
            gpui_kit::init(cx);
            crate::apply_theme(crate::preferences::Theme::Dark, cx);
        });
        let (view, cx) = cx.add_window_view(|_, _| ThemeView(None));
        cx.update(|window, cx| {
            view.update(cx, |view, cx| {
                view.0 = ThemeTransition::begin(crate::preferences::Theme::Light, cx)
            });
            window.draw(cx).clear(cx);
            assert_eq!(ThemeTone::current(cx).0, 1.);
        });
        cx.background_executor
            .advance_clock(Duration::from_millis(120));
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            let tone = ThemeTone::current(cx).0;
            assert!(tone > 0. && tone < 1., "Theme must interpolate");
            let color = ComponentTheme::global(cx).colors.background;
            view.update(cx, |view, cx| {
                view.0 = ThemeTransition::begin(crate::preferences::Theme::Dark, cx)
            });
            assert_eq!(
                ComponentTheme::global(cx).colors.background,
                color,
                "Retarget cannot flash"
            );
            window.draw(cx).clear(cx);
        });
        cx.background_executor
            .advance_clock(Duration::from_millis(400));
        cx.update(|window, cx| {
            window.draw(cx).clear(cx);
            assert!(
                view.read(cx).0.is_none(),
                "Stop scheduling frames when settled"
            );
            assert_eq!(ThemeTone::current(cx).0, 1.);
            cx.set_reduce_motion(true);
            assert!(ThemeTransition::begin(crate::preferences::Theme::Light, cx).is_none());
            assert_eq!(ThemeTone::current(cx).0, 0.);
        });
    }

    #[gpui_kit::test]
    fn switch_accepts_pointer_and_keyboard_and_animates_both_directions(cx: &mut TestAppContext) {
        struct SwitchView(bool);
        impl Render for SwitchView {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div().tab_group().size_full().child(
                    Switch::new("test-switch")
                        .checked(self.0)
                        .label("Startup")
                        .on_click(cx.listener(|view, checked, _, cx| {
                            view.0 = *checked;
                            cx.notify();
                        })),
                )
            }
        }
        cx.update(gpui_kit::init);
        let (view, cx) = cx.add_window_view(|_, _| SwitchView(false));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let start = cx.debug_bounds("test-switch-thumb").unwrap().left();
        let center = cx.debug_bounds("test-switch-track").unwrap().center();
        cx.simulate_click(center, Modifiers::default());
        cx.update(|window, cx| {
            assert!(view.read(cx).0);
            window.draw(cx).clear(cx);
        });
        cx.background_executor
            .advance_clock(Duration::from_millis(80));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let midway = cx.debug_bounds("test-switch-thumb").unwrap().left();
        assert!(midway > start && midway < start + px(12.));
        cx.background_executor
            .advance_clock(Duration::from_millis(200));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(
            cx.debug_bounds("test-switch-thumb").unwrap().left(),
            start + px(12.)
        );
        let keystroke = cx.update(|_, cx| {
            gpui_kit::Keystroke::parse_with_os("space", cx.platform_info().os).unwrap()
        });
        cx.simulate_event(gpui_kit::KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(gpui_kit::KeyUpEvent { keystroke });
        cx.update(|window, cx| {
            assert!(!view.read(cx).0);
            window.draw(cx).clear(cx);
        });
        cx.background_executor
            .advance_clock(Duration::from_millis(200));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(cx.debug_bounds("test-switch-thumb").unwrap().left(), start);
        cx.update(|_, cx| cx.set_reduce_motion(true));
        cx.simulate_click(center, Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(
            cx.debug_bounds("test-switch-thumb").unwrap().left(),
            start + px(12.)
        );
    }

    #[gpui_kit::test]
    fn segmented_switches_animate_independently_and_respect_reduced_motion(
        cx: &mut TestAppContext,
    ) {
        struct Options([usize; 2]);
        impl Render for Options {
            fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
                div()
                    .flex()
                    .flex_col()
                    .children(
                        self.0
                            .iter()
                            .enumerate()
                            .map(|(index, &choice)| SegmentedControl {
                                id: format!("option-{index}").into(),
                                segments: vec![
                                    (format!("option-{index}-off").into(), "Off".into()),
                                    (format!("option-{index}-on").into(), "On".into()),
                                ],
                                selected: Some(choice),
                                accessibility_label: Some("Plugin toggle".into()),
                                width: Some(px(104.)),
                                disabled: false,
                                on_change: Rc::new(cx.listener(
                                    move |view, selected: &usize, _, cx| {
                                        view.0[index] = *selected;
                                        cx.notify();
                                    },
                                )),
                            }),
                    )
            }
        }
        cx.update(gpui_kit::init);
        let (view, cx) = cx.add_window_view(|_, _| Options([0, 1]));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let off = cx.debug_bounds("option-0-off").unwrap();
        let on = cx.debug_bounds("option-0-on").unwrap();
        let other = cx.debug_bounds("option-1-indicator").unwrap();
        cx.simulate_click(on.center(), Modifiers::default());
        cx.update(|window, cx| {
            assert_eq!(view.read(cx).0, [1, 1]);
            window.draw(cx).clear(cx);
        });
        cx.background_executor
            .advance_clock(Duration::from_millis(80));
        cx.update(|window, cx| window.draw(cx).clear(cx));
        let midway = cx.debug_bounds("option-0-indicator").unwrap().left();
        assert!(off.left() < midway && midway < on.left());
        assert_eq!(cx.debug_bounds("option-1-indicator").unwrap(), other);
        cx.update(|_, cx| cx.set_reduce_motion(true));
        cx.simulate_click(off.center(), Modifiers::default());
        cx.update(|window, cx| window.draw(cx).clear(cx));
        assert_eq!(
            cx.debug_bounds("option-0-indicator").unwrap().left(),
            off.left()
        );
    }

    #[test]
    fn retargeting_remains_continuous_and_reduced_motion_settles() {
        let mut state = MotionState::default();
        let motion = |width| Motion::new(Target::default().width(width)).transition(tween(200));
        state.sample(motion(180.), Duration::ZERO, false);
        state.sample(motion(60.), Duration::from_millis(10), false);
        let midway = state.sample(motion(60.), Duration::from_millis(100), false);
        let before = midway.target().width_px().unwrap();
        let retarget = state.sample(motion(180.), Duration::from_millis(100), false);
        assert_eq!(before, retarget.target().width_px().unwrap());
        let end = state.sample(motion(180.), Duration::from_millis(110), true);
        assert_eq!(end.target().width_px(), Some(180.));
        assert!(!end.is_animating());
        assert!(
            !state
                .sample(motion(180.), Duration::from_secs(2), false)
                .is_animating()
        );
    }
}
