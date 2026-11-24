use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

const WINDOW: Duration = Duration::from_secs(4);

/// Recent bytes per second from a cumulative counter and monotonic sample times.
#[derive(Default)]
pub struct SpeedMeter {
    samples: VecDeque<(Instant, u64)>,
}

impl SpeedMeter {
    pub fn reset(&mut self) {
        self.samples.clear();
    }

    pub fn sample(&mut self, now: Instant, bytes: u64) -> u64 {
        if self
            .samples
            .back()
            .is_some_and(|(time, previous)| *time > now || *previous > bytes)
        {
            self.reset();
        }
        if self.samples.back().is_some_and(|&(at, _)| at == now) {
            self.samples.pop_back();
        }
        self.samples.push_back((now, bytes));
        // Keep the observation preceding the window. Dropping it makes a late
        // refresh report zero even if bytes arrived throughout the interval.
        while self.samples.len() >= 2 && now.duration_since(self.samples[1].0) >= WINDOW {
            self.samples.pop_front();
        }
        let (start, previous) = self.samples[0];
        let elapsed = now.duration_since(start);
        if elapsed.is_zero() {
            return 0;
        }
        let received = if elapsed > WINDOW {
            let (next, next_bytes) = self.samples[1];
            // Only this boundary interval straddles the window. Apportion its
            // bytes by observed time, even if the sampler missed several ticks.
            let overlap = WINDOW - (now - next);
            let fraction = overlap.as_secs_f64() / (next - start).as_secs_f64();
            (bytes - next_bytes) as f64 + (next_bytes - previous) as f64 * fraction
        } else {
            (bytes - previous) as f64
        };
        (received / elapsed.min(WINDOW).as_secs_f64()) as u64
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn smooths_bursts_without_inventing_traffic() {
        let start = Instant::now();
        let mut meter = SpeedMeter::default();
        assert_eq!(meter.sample(start, 0), 0);
        assert_eq!(meter.sample(start + Duration::from_secs(1), 1_000), 1_000);
        assert_eq!(
            meter.sample(start + Duration::from_millis(1_500), 1_000),
            666
        );
        assert_eq!(meter.sample(start + Duration::from_secs(5), 1_000), 0);
    }

    #[test]
    fn counter_restart_does_not_create_a_spike() {
        let start = Instant::now();
        let mut meter = SpeedMeter::default();
        meter.sample(start, 5_000);
        assert_eq!(meter.sample(start + Duration::from_secs(1), 100), 0);
        assert_eq!(meter.sample(start + Duration::from_secs(2), 600), 500);
    }

    #[test]
    fn delayed_refresh_still_measures_elapsed_traffic() {
        let start = Instant::now();
        let mut meter = SpeedMeter::default();
        meter.sample(start, 50_000);
        for millis in [100, 200, 2_260, 2_390, 6_800, 12_100, 12_250] {
            let speed = meter.sample(start + Duration::from_millis(millis), 50_000 + millis * 10);
            assert!(speed.abs_diff(10_000) <= 1, "{speed} B/s at {millis} ms");
        }
    }

    #[test]
    fn batched_progress_is_stable_across_ui_and_scheduler_cadences_then_stalls() {
        let start = Instant::now();
        for tick_ms in [100, 1_000] {
            let mut meter = SpeedMeter::default();
            for millis in (0..=16_000).step_by(tick_ms) {
                // 2,000 bytes arrive every two seconds until the 12s mark.
                let bytes = millis.min(12_000) / 2_000 * 2_000;
                let speed = meter.sample(start + Duration::from_millis(millis), bytes);
                if (4_000..=12_000).contains(&millis) {
                    assert_eq!(speed, 1_000, "{tick_ms}ms sampler at {millis}ms");
                }
                if millis == 16_000 {
                    assert_eq!(speed, 0);
                }
            }
        }
    }
}
