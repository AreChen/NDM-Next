pub(crate) const DEFAULT_WIDTH: f32 = 180.;
pub(crate) const MIN_WIDTH: f32 = 160.;
pub(crate) const MAX_WIDTH: f32 = 320.;
const COLLAPSED_WIDTH: f32 = 60.;
const COLLAPSE_AT: f32 = 120.;
const EXPAND_AT: f32 = 152.;

pub(crate) fn valid_width(width: f32) -> f32 {
    if width.is_finite() {
        width.clamp(MIN_WIDTH, MAX_WIDTH)
    } else {
        DEFAULT_WIDTH
    }
}

pub(crate) struct Sidebar {
    pub collapsed: bool,
    pub width: f32,
    drag: Option<(f32, f32)>,
}

impl Sidebar {
    pub fn new(collapsed: bool, width: f32) -> Self {
        Self {
            collapsed,
            width: valid_width(width),
            drag: None,
        }
    }

    pub fn display_width(&self) -> f32 {
        if self.collapsed {
            COLLAPSED_WIDTH
        } else {
            self.width
        }
    }

    pub fn is_resizing(&self) -> bool {
        self.drag.is_some()
    }

    pub fn begin(&mut self, pointer_x: f32) {
        self.drag = Some((pointer_x, self.display_width()));
    }

    pub fn resize(&mut self, pointer_x: f32) -> bool {
        let Some((start, width)) = self.drag else {
            return false;
        };
        let requested = width + pointer_x - start;
        if !requested.is_finite() {
            return false;
        }
        let before = (self.collapsed, self.width);
        if requested <= COLLAPSE_AT {
            self.collapsed = true;
        } else if !self.collapsed || requested >= EXPAND_AT {
            self.collapsed = false;
            self.width = valid_width(requested);
        }
        before != (self.collapsed, self.width)
    }

    pub fn finish(&mut self) -> bool {
        self.drag.take().is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resizing_collapses_with_hysteresis_and_restores_the_last_expanded_width() {
        let mut side = Sidebar::new(false, 220.);
        side.begin(220.);
        assert!(side.resize(280.));
        assert_eq!(side.display_width(), 280.);
        side.resize(110.);
        assert!(side.collapsed);
        assert_eq!(side.display_width(), 60.);
        side.resize(140.);
        assert!(side.collapsed);
        side.resize(180.);
        assert!(!side.collapsed);
        assert_eq!(side.width, 180.);
        side.resize(100.);
        assert!(side.finish());
        assert!(!side.finish());
        side.collapsed = false;
        assert_eq!(side.width, 180.);
        assert!(!side.resize(300.));
    }

    #[test]
    fn collapsed_handle_expands_and_extreme_widths_are_bounded() {
        let mut side = Sidebar::new(true, f32::NAN);
        side.begin(60.);
        side.resize(200.);
        assert!(!side.collapsed);
        assert_eq!(side.width, 200.);
        side.resize(10_000.);
        assert_eq!(side.width, MAX_WIDTH);
        assert!(!side.resize(f32::NAN));
        assert_eq!(Sidebar::new(false, -10.).width, MIN_WIDTH);
    }
}
