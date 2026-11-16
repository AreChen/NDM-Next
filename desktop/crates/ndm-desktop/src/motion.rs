use gpui_motion_kit::core::{Easing, Transition};
use std::time::Duration;

pub(crate) fn tween(milliseconds: u64) -> Transition {
    Transition::Tween(Duration::from_millis(milliseconds), Easing::EaseOut)
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui_motion_kit::{
        MotionState,
        core::{Motion, Target},
    };

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
