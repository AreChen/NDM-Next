use crate::{
    language::Language,
    motion::SegmentedControl,
    palette::{CustomPalette, DownloadColors, DownloadStatus, DownloadStyle, ThemeTone, rgb_value},
    preferences::ColorScheme,
};
use gpui_kit::{
    component::{
        Disableable, Selectable, Sizable, Theme,
        button::{Button, ButtonVariants},
        input::{Input, InputEvent, InputState},
        popover::Popover,
        slider::{Slider, SliderEvent, SliderState},
    },
    prelude::*,
    *,
};
use std::{cell::Cell, rc::Rc};

#[derive(Clone, Copy, PartialEq, Eq)]
enum ColorRole {
    Primary,
    Secondary,
    Surface,
    Download(DownloadStatus),
}

impl ColorRole {
    fn label(self, language: Language) -> &'static str {
        match self {
            Self::Primary => language.text("主题色", "Theme color"),
            Self::Secondary => language.text("辅助色", "Secondary"),
            Self::Surface => language.text("界面底色", "Surface color"),
            Self::Download(status) => status.label(language),
        }
    }

    fn color(self, palette: PaletteSettings) -> u32 {
        match self {
            Self::Primary => palette.interface.primary,
            Self::Secondary => palette.interface.accents()[1],
            Self::Surface => palette.interface.surface(),
            Self::Download(status) => palette
                .downloads
                .get(status)
                .unwrap_or(status.default_color()),
        }
    }

    fn can_reset(self, palette: PaletteSettings) -> bool {
        match self {
            Self::Primary => false,
            Self::Secondary => palette.interface.secondary.is_some(),
            Self::Surface => {
                palette.interface.surface.is_some() || palette.interface.legacy_surfaces
            }
            Self::Download(status) => palette.downloads.get(status).is_some(),
        }
    }

    fn section(self) -> PaletteSection {
        match self {
            Self::Download(_) => PaletteSection::Downloads,
            _ => PaletteSection::Interface,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PaletteSection {
    Interface,
    Downloads,
}

impl PaletteSection {
    const ALL: [Self; 2] = [Self::Interface, Self::Downloads];

    fn label(self, language: Language) -> &'static str {
        match self {
            Self::Interface => language.text("界面颜色", "Interface colors"),
            Self::Downloads => language.text("下载状态", "Download statuses"),
        }
    }

    fn roles(self) -> &'static [ColorRole] {
        match self {
            Self::Interface => &[ColorRole::Primary, ColorRole::Secondary, ColorRole::Surface],
            Self::Downloads => &[
                ColorRole::Download(DownloadStatus::Completed),
                ColorRole::Download(DownloadStatus::Failed),
                ColorRole::Download(DownloadStatus::Paused),
                ColorRole::Download(DownloadStatus::Downloading),
                ColorRole::Download(DownloadStatus::Queued),
            ],
        }
    }
}

#[derive(Clone, Copy)]
struct Hsv {
    hue: f32,
    saturation: f32,
    value: f32,
}

impl Hsv {
    fn from_rgb(color: u32) -> Self {
        let color: Hsla = rgb(color).into();
        let chroma_half = color.s * color.l.min(1. - color.l);
        let value = color.l + chroma_half;
        Self {
            hue: color.h,
            saturation: if value > 0. {
                2. * chroma_half / value
            } else {
                0.
            },
            value,
        }
    }

    fn color(self) -> Hsla {
        let lightness = self.value * (1. - self.saturation / 2.);
        let range = lightness.min(1. - lightness);
        let saturation = if range > 0. {
            self.value * self.saturation / (2. * range)
        } else {
            0.
        };
        hsla(self.hue, saturation.clamp(0., 1.), lightness, 1.)
    }
}

#[derive(Clone)]
struct PlaneDrag(EntityId);

impl Render for PlaneDrag {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        Empty
    }
}

#[derive(Clone, Copy, Default)]
pub(super) struct PaletteSettings {
    pub interface: CustomPalette,
    pub downloads: DownloadColors,
}

#[derive(Clone, Copy)]
pub(super) enum ApplyPalette {
    Interface(CustomPalette),
    Downloads(DownloadColors),
}

pub(super) struct ColorEditor {
    draft: PaletteSettings,
    saved: PaletteSettings,
    section: PaletteSection,
    role: Option<ColorRole>,
    color: Hsv,
    hue: Entity<SliderState>,
    hex: Entity<InputState>,
    valid_hex: bool,
    plane_focus: FocusHandle,
    language: Language,
}

impl EventEmitter<ApplyPalette> for ColorEditor {}

impl ColorEditor {
    pub(super) fn new(
        palette: PaletteSettings,
        language: Language,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let seed = palette.interface.primary;
        let color = Hsv::from_rgb(seed);
        let hue = cx.new(|_| {
            SliderState::new()
                .max(360.)
                .step(1.)
                .default_value(color.hue * 360.)
        });
        let hex = cx.new(|cx| InputState::new(window, cx).default_value(format!("#{seed:06X}")));
        cx.subscribe_in(&hue, window, |this, _, event, window, cx| {
            if this.role.is_some()
                && let SliderEvent::Change(value) = event
            {
                this.color.hue = value.end() / 360.;
                this.update_from_picker(window, cx);
            }
        })
        .detach();
        cx.subscribe_in(&hex, window, |this, hex, event, window, cx| {
            if this.role.is_some()
                && let InputEvent::Change = event
            {
                let color = parse_hex(&hex.read(cx).value());
                this.valid_hex = color.is_some();
                if let Some(color) = color {
                    this.color = Hsv::from_rgb(color);
                    this.sync_hue(window, cx);
                    this.edit_color(cx);
                }
                cx.notify();
            }
        })
        .detach();
        Self {
            draft: palette,
            saved: palette,
            section: PaletteSection::Interface,
            role: None,
            color,
            hue,
            hex,
            valid_hex: true,
            plane_focus: cx.focus_handle().tab_stop(true),
            language,
        }
    }

    pub(super) fn set_language(&mut self, language: Language, cx: &mut Context<Self>) {
        self.language = language;
        cx.notify();
    }

    pub(super) fn mark_saved(&mut self, palette: ApplyPalette, cx: &mut Context<Self>) {
        match palette {
            ApplyPalette::Interface(palette) => self.saved.interface = palette,
            ApplyPalette::Downloads(colors) => self.saved.downloads = colors,
        }
        self.dismiss(cx);
        cx.notify();
    }

    pub(super) fn dismiss(&mut self, cx: &mut Context<Self>) {
        if self.role.take().is_some() {
            self.draft = self.saved;
            cx.notify();
        }
    }

    fn select_section(&mut self, section: PaletteSection, cx: &mut Context<Self>) {
        if self.section != section {
            self.dismiss(cx);
            self.section = section;
            cx.notify();
        }
    }

    fn set_open(
        &mut self,
        role: ColorRole,
        open: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if open && self.role != Some(role) {
            self.draft = self.saved;
            self.load_role(role, window, cx);
        } else if !open && self.role == Some(role) {
            self.dismiss(cx);
        }
    }

    fn changes(&self) -> Option<ApplyPalette> {
        if !self.valid_hex {
            return None;
        }
        match self.role?.section() {
            PaletteSection::Interface => (self.draft.interface != self.saved.interface)
                .then_some(ApplyPalette::Interface(self.draft.interface)),
            PaletteSection::Downloads => (self.draft.downloads != self.saved.downloads)
                .then_some(ApplyPalette::Downloads(self.draft.downloads)),
        }
    }

    fn load_role(&mut self, role: ColorRole, window: &mut Window, cx: &mut Context<Self>) {
        self.role = Some(role);
        self.color = Hsv::from_rgb(role.color(self.draft));
        self.sync_hue(window, cx);
        self.sync_hex(window, cx);
        cx.notify();
    }

    fn sync_hue(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.hue.update(cx, |state, cx| {
            state.set_value(self.color.hue * 360., window, cx)
        });
    }

    fn sync_hex(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(role) = self.role else { return };
        self.valid_hex = true;
        let color = role.color(self.draft);
        self.hex.update(cx, |input, cx| {
            input.set_value(format!("#{color:06X}"), window, cx)
        });
    }

    fn update_from_picker(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.edit_color(cx);
        self.sync_hex(window, cx);
    }

    fn edit_color(&mut self, cx: &mut Context<Self>) {
        let Some(role) = self.role else { return };
        let color = rgb_value(self.color.color());
        match role {
            ColorRole::Primary if self.draft.interface.primary != color => {
                self.draft.interface.primary = color;
                self.draft.interface.legacy_surfaces = false;
            }
            ColorRole::Primary => {}
            ColorRole::Secondary => self.draft.interface.secondary = Some(color),
            ColorRole::Surface => {
                self.draft.interface.surface = Some(color);
                self.draft.interface.legacy_surfaces = false;
            }
            ColorRole::Download(status) => self.draft.downloads.set(status, Some(color)),
        }
        cx.notify();
    }

    fn reset_role(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(role) = self.role else { return };
        match role {
            ColorRole::Primary => return,
            ColorRole::Secondary => self.draft.interface.secondary = None,
            ColorRole::Surface => {
                self.draft.interface.surface = None;
                self.draft.interface.legacy_surfaces = false;
            }
            ColorRole::Download(status) => self.draft.downloads.set(status, None),
        }
        self.load_role(role, window, cx);
    }

    fn pick_position(
        &mut self,
        position: Point<Pixels>,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if bounds.size.width <= px(0.) || bounds.size.height <= px(0.) {
            return;
        }
        self.color.saturation = ((position.x - bounds.left()) / bounds.size.width).clamp(0., 1.);
        self.color.value = ((bounds.bottom() - position.y) / bounds.size.height).clamp(0., 1.);
        self.update_from_picker(window, cx);
    }

    fn plane(&self, window: &Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focus = self.plane_focus.is_focused(window);
        let color = self.color;
        let bounds = Rc::new(Cell::new(Bounds::default()));
        let hitbox = bounds.clone();
        let id = cx.entity_id();
        let label = self
            .language
            .text("饱和度与明度", "Saturation and brightness");
        div()
            .id("custom-color-plane")
            .debug_selector(|| "custom-color-plane".into())
            .track_focus(&self.plane_focus)
            .role(Role::Group)
            .aria_label(label)
            .cursor_crosshair()
            .relative()
            .h(px(118.))
            .w_full()
            .child(
                div()
                    .absolute()
                    .size_full()
                    .rounded_md()
                    .overflow_hidden()
                    .bg(hsla(color.hue, 1., 0.5, 1.))
                    .child(div().absolute().size_full().bg(linear_gradient(
                        90.,
                        linear_color_stop(rgb(0xffffff), 0.),
                        linear_color_stop(rgba(0xffffff00), 1.),
                    )))
                    .child(div().absolute().size_full().bg(linear_gradient(
                        180.,
                        linear_color_stop(rgba(0x00000000), 0.),
                        linear_color_stop(rgb(0x000000), 1.),
                    ))),
            )
            .child(
                canvas(move |rect, _, _| bounds.set(rect), |_, _, _, _| {})
                    .absolute()
                    .size_full(),
            )
            .child(
                div()
                    .absolute()
                    .left(relative(color.saturation))
                    .top(relative(1. - color.value))
                    .ml(-px(7.))
                    .mt(-px(7.))
                    .size(px(14.))
                    .rounded_full()
                    .bg(color.color())
                    .border_2()
                    .border_color(rgb(0xffffff))
                    .shadow_md(),
            )
            .when(focus, |plane| {
                plane.child(
                    div()
                        .absolute()
                        .size_full()
                        .rounded_md()
                        .border_2()
                        .border_color(rgb(0xffffff)),
                )
            })
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, window, cx| {
                    this.plane_focus.focus(window, cx);
                    this.pick_position(event.position, hitbox.get(), window, cx);
                }),
            )
            .on_drag(PlaneDrag(id), |drag, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| drag.clone())
            })
            .on_drag_move(
                cx.listener(move |this, event: &DragMoveEvent<PlaneDrag>, window, cx| {
                    if event.drag(cx).0 == id {
                        this.pick_position(event.event.position, event.bounds, window, cx);
                    }
                }),
            )
            .on_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                let step = if event.keystroke.modifiers.shift {
                    0.10
                } else {
                    0.02
                };
                match event.keystroke.key.as_str() {
                    "left" => this.color.saturation -= step,
                    "right" => this.color.saturation += step,
                    "up" => this.color.value += step,
                    "down" => this.color.value -= step,
                    _ => return,
                }
                this.color.saturation = this.color.saturation.clamp(0., 1.);
                this.color.value = this.color.value.clamp(0., 1.);
                this.update_from_picker(window, cx);
                cx.stop_propagation();
            }))
    }

    fn preview(&self, role: ColorRole, tone: &ThemeTone, dark: bool) -> Div {
        let (background, edge, accent, ink, bar, label) = match role {
            ColorRole::Download(status) => {
                let style = DownloadStyle::new(tone, status, self.draft.downloads.get(status));
                (
                    style.row_background(),
                    style.edge,
                    style.accent,
                    style.accent,
                    style.progress_background(),
                    status.label(self.language),
                )
            }
            _ => {
                let preview = ThemeTone::new(
                    dark,
                    ColorScheme::Custom,
                    self.draft.interface,
                    self.saved.downloads,
                );
                (
                    rgb(preview.surface(0x0c1120, 0xf4f6fb)).into(),
                    preview.rgb(0x354260, 0xdce1f0),
                    preview.rgb(0xa488ff, 0x6544b4),
                    preview.rgb(0xf0f3ff, 0x19253b),
                    linear_gradient(
                        90.,
                        linear_color_stop(rgb(preview.rgb(0x885dff, 0x885dff)), 0.),
                        linear_color_stop(rgb(preview.rgb(0x37c9f5, 0x37c9f5)), 1.),
                    ),
                    self.language.text("预览", "Preview"),
                )
            }
        };
        div()
            .w_full()
            .h(px(32.))
            .px_2()
            .rounded_md()
            .border_1()
            .border_color(rgb(edge))
            .bg(background)
            .flex()
            .items_center()
            .gap_2()
            .child(swatch(accent, edge, px(12.)))
            .child(
                div()
                    .flex_1()
                    .min_w(px(14.))
                    .h(px(5.))
                    .rounded_full()
                    .bg(bar),
            )
            .child(
                div()
                    .min_w_0()
                    .text_xs()
                    .truncate()
                    .text_color(rgb(ink))
                    .child(label),
            )
    }
}

fn parse_hex(value: &str) -> Option<u32> {
    let value = value.trim();
    let hex = value.strip_prefix('#').unwrap_or(value);
    if hex.len() != 6 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()
}

fn swatch(color: u32, border: u32, size: Pixels) -> Div {
    div()
        .size(size)
        .flex_shrink_0()
        .rounded_full()
        .border_1()
        .border_color(rgb(border))
        .bg(rgb(color))
}

impl ColorEditor {
    fn picker(
        &self,
        role: ColorRole,
        tone: &ThemeTone,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Stateful<Div> {
        let language = self.language;
        let dark = Theme::global(cx).mode.is_dark();
        let hue_label = language.text("色相", "Hue");
        let spectrum = div()
            .absolute()
            .left_0()
            .right_0()
            .h(px(10.))
            .flex()
            .rounded_full()
            .overflow_hidden()
            .children((0..6).map(|index| {
                div().flex_1().h_full().bg(linear_gradient(
                    90.,
                    linear_color_stop(hsla(index as f32 / 6., 1., 0.5, 1.), 0.),
                    linear_color_stop(hsla((index + 1) as f32 / 6., 1., 0.5, 1.), 1.),
                ))
            }));

        div()
            .id("custom-color-picker")
            .debug_selector(|| "custom-color-picker".into())
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(role.label(language)),
                    )
                    .child(
                        div().w(px(88.)).flex_shrink_0().child(
                            Input::new(&self.hex)
                                .id("custom-color-hex")
                                .aria_label(language.text("十六进制颜色", "Hex color"))
                                .small()
                                .font_family("monospace"),
                        ),
                    ),
            )
            .child(self.plane(window, cx))
            .child(
                div()
                    .relative()
                    .flex()
                    .items_center()
                    .mx_2()
                    .h_5()
                    .child(spectrum)
                    .child(
                        Slider::new(&self.hue)
                            .flex_1()
                            .bg(hsla(0., 0., 0., 0.))
                            .text_color(rgb(0xf6f7fa))
                            .accessibility_label(hue_label),
                    ),
            )
            .child(self.preview(role, tone, dark))
            .child(
                div()
                    .flex()
                    .items_center()
                    .gap_2()
                    .when(role.can_reset(self.draft), |row| {
                        row.child(
                            Button::new("auto-color")
                                .small()
                                .ghost()
                                .debug_selector(|| "auto-color".into())
                                .label(match role.section() {
                                    PaletteSection::Interface => {
                                        language.text("恢复自动配色", "Use automatic color")
                                    }
                                    PaletteSection::Downloads => {
                                        language.text("恢复默认颜色", "Restore default color")
                                    }
                                })
                                .on_click(
                                    cx.listener(|this, _, window, cx| this.reset_role(window, cx)),
                                ),
                        )
                    })
                    .child(
                        Button::new("apply-custom-colors")
                            .small()
                            .primary()
                            .ml_auto()
                            .debug_selector(|| "apply-custom-colors".into())
                            .disabled(self.changes().is_none())
                            .label(language.text("应用", "Apply"))
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(change) = this.changes() {
                                    cx.emit(change);
                                }
                            })),
                    ),
            )
    }

    fn swatch_tile(
        &self,
        index: usize,
        role: ColorRole,
        tone: &ThemeTone,
        window: &Window,
        cx: &mut Context<Self>,
    ) -> Popover {
        let label = role.label(self.language);
        let open = self.role == Some(role);
        let button = Button::new(("custom-color-role", index))
            .small()
            .outline()
            .w_full()
            .h(px(30.))
            .min_w_0()
            .debug_selector(move || format!("custom-color-role-{index}"))
            .selected(open)
            .accessibility_label(label)
            .when(!open, |button| button.tooltip(label))
            .child(
                div()
                    .flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_1p5()
                    .child(
                        swatch(
                            role.color(self.saved),
                            tone.rgb(0x566179, 0xb6bfcd),
                            px(14.),
                        )
                        .rounded(px(4.)),
                    )
                    .child(div().min_w_0().truncate().child(label)),
            );
        Popover::new(("custom-color-popover", index))
            .open(open)
            .anchor(Anchor::BottomLeft)
            .offset(px(6.))
            .w(px(332.))
            .max_w((window.viewport_size().width - px(16.)).max(px(1.)))
            .rounded_lg()
            .bg(rgb(tone.surface(0x192033, 0xfdfdff)))
            .border_color(rgb(tone.rgb(0x46516e, 0xd8deea)))
            .trigger_style(StyleRefinement::default().min_w_0().w_full())
            .trigger(button)
            .track_focus(&self.plane_focus)
            .on_open_change(cx.listener(move |this, open, window, cx| {
                this.set_open(role, *open, window, cx);
            }))
            .when(open, |popover| {
                popover.child(self.picker(role, tone, window, cx))
            })
    }
}

impl Render for ColorEditor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let language = self.language;
        let tone = ThemeTone::current(cx);
        let view = cx.entity().downgrade();
        let sections = SegmentedControl {
            id: "custom-color-sections".into(),
            segments: [
                ("custom-color-interface", PaletteSection::Interface),
                ("custom-color-downloads", PaletteSection::Downloads),
            ]
            .into_iter()
            .map(|(id, section)| (id.into(), section.label(language).into()))
            .collect(),
            selected: Some(self.section as usize),
            accessibility_label: Some(language.text("界面配色", "Color palette").into()),
            width: None,
            disabled: false,
            on_change: Rc::new(move |index, _, cx| {
                let _ = view.update(cx, |this, cx| {
                    this.select_section(PaletteSection::ALL[*index], cx);
                });
            }),
        };

        div()
            .id("custom-color-editor")
            .debug_selector(|| "custom-color-editor".into())
            .w_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .rounded_lg()
            .border_1()
            .border_color(rgb(tone.rgb(0x354260, 0xd5dce9)))
            .bg(rgb(tone.surface(0x151b2b, 0xf8f9fc)))
            .child(div().flex().child(sections))
            .child(
                div().w_full().grid().grid_cols(3).gap_1p5().children(
                    self.section
                        .roles()
                        .iter()
                        .copied()
                        .enumerate()
                        .map(|(index, role)| self.swatch_tile(index, role, &tone, window, cx)),
                ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ApplyPalette, ColorEditor, CustomPalette, Hsv, Language, PaletteSettings, rgb_value,
    };
    use crate::palette::{DownloadColors, DownloadStatus};
    use gpui_kit::{
        self as gpui, Modifiers, MouseButton, TestAppContext, VisualTestContext, point, px, size,
    };
    use std::{cell::Cell, rc::Rc};

    fn redraw(cx: &mut VisualTestContext) {
        cx.update(|window, cx| window.draw(cx).clear(cx));
    }

    fn click(cx: &mut VisualTestContext, selector: &'static str) {
        let bounds = cx
            .debug_bounds(selector)
            .unwrap_or_else(|| panic!("{selector} is missing"));
        cx.simulate_click(bounds.center(), Modifiers::default());
        redraw(cx);
    }

    #[test]
    fn hsv_round_trips_rgb_swatches_and_extremes() {
        for color in [
            0, 0xffffff, 0xff0000, 0x00ff00, 0x0000ff, 0x885dff, 0x637c32,
        ] {
            assert_eq!(rgb_value(Hsv::from_rgb(color).color()), color);
        }
    }

    #[gpui::test]
    fn swatch_opens_a_picker_without_resizing_the_editor(cx: &mut TestAppContext) {
        cx.update(gpui::init);
        let applied = Rc::new(Cell::new(None));
        let picked = applied.clone();
        let (editor, cx) = cx.add_window_view(move |window, cx| -> ColorEditor {
            cx.subscribe(&cx.entity(), move |editor, _, change: &ApplyPalette, cx| {
                picked.set(Some(*change));
                editor.mark_saved(*change, cx);
            })
            .detach();
            ColorEditor::new(PaletteSettings::default(), Language::German, window, cx)
        });
        cx.simulate_resize(size(px(365.), px(400.)));
        redraw(cx);
        let compact_editor = cx.debug_bounds("custom-color-editor").unwrap();
        assert!(cx.debug_bounds("custom-color-picker").is_none());
        assert!(cx.debug_bounds("custom-color-role-2").is_some());
        assert!(cx.debug_bounds("custom-color-role-3").is_none());

        click(cx, "custom-color-role-0");
        let plane = cx.debug_bounds("custom-color-plane").unwrap();
        assert_eq!(cx.debug_bounds("custom-color-editor"), Some(compact_editor));
        let start = point(
            plane.left() + plane.size.width * 0.2,
            plane.top() + plane.size.height * 0.35,
        );
        let end = point(
            plane.left() + plane.size.width * 0.65,
            plane.top() + plane.size.height * 0.25,
        );
        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::default());
        let clicked = cx.update(|_, cx| editor.read(cx).draft.interface);
        assert_ne!(clicked.primary, CustomPalette::default().primary);
        assert_ne!(clicked.accents()[1], CustomPalette::default().accents()[1]);
        assert_ne!(clicked.surface(), CustomPalette::default().surface());
        cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::default());
        cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::default());
        assert_ne!(
            cx.update(|_, cx| editor.read(cx).draft.interface.primary),
            clicked.primary
        );
        let before_keys = cx.update(|_, cx| editor.read(cx).draft.interface.primary);
        cx.simulate_keystrokes("left up");
        assert_ne!(
            cx.update(|_, cx| editor.read(cx).draft.interface.primary),
            before_keys
        );

        cx.update(|window, cx| {
            let input = editor.read(cx).hex.clone();
            gpui::Focusable::focus_handle(input.read(cx), cx).focus(window, cx);
        });
        cx.simulate_keystrokes(if cfg!(target_os = "macos") {
            "cmd-a"
        } else {
            "ctrl-a"
        });
        cx.simulate_input("#E45A6F");
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).draft.interface.primary),
            0xe45a6f
        );
        assert!(applied.get().is_none());
        redraw(cx);
        assert_eq!(cx.debug_bounds("custom-color-plane"), Some(plane));
        click(cx, "apply-custom-colors");
        let Some(ApplyPalette::Interface(saved)) = applied.get() else {
            panic!("Apply should save only the interface palette");
        };
        assert_eq!(saved.primary, 0xe45a6f);
        assert!(saved.secondary.is_none() && saved.surface.is_none());
        assert!(cx.debug_bounds("custom-color-picker").is_none());
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).saved.downloads),
            DownloadColors::default()
        );
    }

    #[gpui::test]
    fn canceling_interface_edits_keeps_status_changes_independent(cx: &mut TestAppContext) {
        cx.update(gpui::init);
        let applied = Rc::new(Cell::new(None));
        let picked = applied.clone();
        let (editor, cx) = cx.add_window_view(move |window, cx| -> ColorEditor {
            cx.subscribe(&cx.entity(), move |editor, _, change: &ApplyPalette, cx| {
                picked.set(Some(*change));
                editor.mark_saved(*change, cx);
            })
            .detach();
            ColorEditor::new(
                PaletteSettings::default(),
                Language::BrazilianPortuguese,
                window,
                cx,
            )
        });
        cx.simulate_resize(size(px(320.), px(405.)));
        redraw(cx);
        click(cx, "custom-color-role-1");
        click(cx, "custom-color-plane");
        assert!(cx.update(|_, cx| editor.read(cx).draft.interface.secondary.is_some()));
        cx.simulate_keystrokes("escape");
        redraw(cx);
        assert!(cx.debug_bounds("custom-color-picker").is_none());
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).draft.interface),
            CustomPalette::default()
        );

        click(cx, "custom-color-downloads");
        let compact_editor = cx.debug_bounds("custom-color-editor").unwrap();
        let top = cx.debug_bounds("custom-color-role-0").unwrap();
        let bottom = cx.debug_bounds("custom-color-role-3").unwrap();
        assert_eq!(top.left(), bottom.left());
        assert!(top.bottom() < bottom.top());
        assert!(cx.debug_bounds("custom-color-role-4").is_some());
        assert!(cx.debug_bounds("custom-color-picker").is_none());
        click(cx, "custom-color-role-2");
        let popup = cx.debug_bounds("custom-color-picker").unwrap();
        assert!(popup.left() >= px(0.) && popup.right() <= px(320.));
        assert!(popup.top() >= px(0.) && popup.bottom() < px(405.));
        let plane = cx.debug_bounds("custom-color-plane").unwrap();
        click(cx, "custom-color-plane");
        assert_eq!(cx.debug_bounds("custom-color-plane"), Some(plane));
        assert_eq!(cx.debug_bounds("custom-color-editor"), Some(compact_editor));

        cx.simulate_click(point(px(10.), px(390.)), Modifiers::default());
        redraw(cx);
        assert!(cx.debug_bounds("custom-color-picker").is_none());
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).draft.downloads),
            DownloadColors::default()
        );
        assert!(applied.get().is_none());

        click(cx, "custom-color-role-2");
        click(cx, "custom-color-plane");
        click(cx, "apply-custom-colors");
        let Some(ApplyPalette::Downloads(colors)) = applied.get() else {
            panic!("Apply should save only download status colors");
        };
        assert!(colors.get(DownloadStatus::Paused).is_some());
        assert!(colors.get(DownloadStatus::Completed).is_none());
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).saved.interface),
            CustomPalette::default()
        );
        click(cx, "custom-color-role-2");
        click(cx, "auto-color");
        assert!(cx.debug_bounds("auto-color").is_none());
        click(cx, "apply-custom-colors");
        assert_eq!(
            cx.update(|_, cx| editor.read(cx).saved.downloads),
            DownloadColors::default()
        );
    }
}
