use gpui_motion_kit::{
    MotionState,
    core::{Easing, Motion, Target, Transition},
};
use std::time::Duration;

pub(crate) fn tween(milliseconds: u64) -> Transition {
    Transition::Tween(Duration::from_millis(milliseconds), Easing::EaseOut)
}

/// Sampled by the existing progress poll only; never owns a timer or requests frames.
#[derive(Default)]
pub(crate) struct Ambient {
    state: MotionState,
}

impl Ambient {
    pub(crate) fn sample(&mut self, elapsed: Duration, enabled: bool) -> Option<f32> {
        if !enabled {
            return None;
        }
        let target = if (elapsed.as_secs() / 8).is_multiple_of(2) {
            1.
        } else {
            -0.6
        };
        let frame = self.state.sample(
            Motion::new(Target::default().x(target))
                .initial(Target::default().x(-0.6))
                .transition(Transition::Tween(Duration::from_secs(8), Easing::EaseInOut)),
            elapsed,
            false,
        );
        Some(frame.target().x_px())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn ambient_is_bounded_and_absent_when_disabled_or_inactive() {
        let mut ambient = Ambient::default();
        assert_eq!(ambient.sample(Duration::ZERO, false), None);
        for tick in 0..400 {
            let x = ambient
                .sample(Duration::from_millis(tick * 100), true)
                .unwrap();
            assert!((-0.601..=1.001).contains(&x));
        }
        assert_eq!(ambient.sample(Duration::from_secs(40), false), None);
    }
}
