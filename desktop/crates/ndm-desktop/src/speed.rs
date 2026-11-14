use std::{
    collections::VecDeque,
    time::{Duration, Instant},
};

/// Rolling average of actual bytes, independent of the refresh interval.
#[derive(Default)]
pub(crate) struct SpeedMeter {
    samples: VecDeque<(Instant, u64)>,
}

impl SpeedMeter {
    pub(crate) fn reset(&mut self) {
        self.samples.clear();
    }

    pub(crate) fn sample(&mut self, now: Instant, bytes: u64) -> u64 {
        if self
            .samples
            .back()
            .is_some_and(|(time, previous)| *time > now || *previous > bytes)
        {
            self.reset();
        }
        self.samples.push_back((now, bytes));
        while self.samples.len() > 1
            && now.duration_since(self.samples[0].0) > Duration::from_millis(1_500)
        {
            self.samples.pop_front();
        }
        let Some(&(start, previous)) = self.samples.front() else {
            return 0;
        };
        let elapsed = now.duration_since(start).as_secs_f64();
        if elapsed <= 0.0 {
            0
        } else {
            (bytes.saturating_sub(previous) as f64 / elapsed) as u64
        }
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
        assert_eq!(meter.sample(start + Duration::from_secs(3), 1_000), 0);
    }

    #[test]
    fn counter_restart_does_not_create_a_spike() {
        let start = Instant::now();
        let mut meter = SpeedMeter::default();
        meter.sample(start, 5_000);
        assert_eq!(meter.sample(start + Duration::from_secs(1), 100), 0);
        assert_eq!(meter.sample(start + Duration::from_secs(2), 600), 500);
    }
}
