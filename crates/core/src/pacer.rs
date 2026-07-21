use std::time::{Duration, Instant};

#[derive(Clone, Debug)]
pub struct FramePacer {
    interval: Duration,
    next_deadline: Instant,
}

impl FramePacer {
    pub fn new(start: Instant, interval: Duration) -> Self {
        assert!(!interval.is_zero(), "frame interval must be non-zero");
        Self {
            interval,
            next_deadline: start,
        }
    }

    pub fn deadline(&self) -> Instant {
        self.next_deadline
    }

    pub fn wait_duration(&self, now: Instant) -> Duration {
        self.next_deadline.saturating_duration_since(now)
    }

    pub fn is_due(&self, now: Instant, early_tolerance: Duration) -> bool {
        now.checked_add(early_tolerance)
            .is_none_or(|with_tolerance| with_tolerance >= self.next_deadline)
    }

    pub fn advance(&mut self, now: Instant) -> u64 {
        let mut skipped = 0;
        loop {
            self.next_deadline += self.interval;
            if self.next_deadline > now {
                return skipped;
            }
            skipped += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadlines_do_not_drift_after_late_work() {
        let start = Instant::now();
        let interval = Duration::from_millis(10);
        let mut pacer = FramePacer::new(start, interval);
        assert!(pacer.is_due(start, Duration::ZERO));
        assert_eq!(pacer.advance(start + Duration::from_millis(7)), 0);
        assert_eq!(pacer.deadline(), start + interval);
        assert_eq!(pacer.advance(start + Duration::from_millis(35)), 2);
        assert_eq!(pacer.deadline(), start + Duration::from_millis(40));
    }

    #[test]
    fn early_tolerance_accepts_display_aligned_arrivals() {
        let start = Instant::now();
        let interval = Duration::from_nanos(1_000_000_000 / 30);
        let pacer = FramePacer::new(start + interval, interval);
        let slightly_early = start + interval - Duration::from_micros(500);
        assert!(!pacer.is_due(slightly_early, Duration::ZERO));
        assert!(pacer.is_due(slightly_early, Duration::from_millis(1)));
    }

    #[test]
    fn decimates_common_display_rates_to_thirty_frames_per_second() {
        let output_interval = Duration::from_nanos(1_000_000_000 / 30);
        for input_interval in [
            Duration::from_nanos(1_000_000_000 / 60),
            Duration::from_nanos(1_000_000_000 * 1_001 / 60_000),
            output_interval,
        ] {
            let start = Instant::now();
            let mut pacer = FramePacer::new(start, output_interval);
            let mut accepted = 0;
            for index in 0..300 {
                let now = start + input_interval * index;
                if pacer.is_due(now, Duration::from_millis(1)) {
                    pacer.advance(now);
                    accepted += 1;
                }
            }
            let elapsed = input_interval * 299;
            let expected = (elapsed.as_secs_f64() * 30.0).round() as i32 + 1;
            assert!(
                (accepted - expected).abs() <= 1,
                "accepted {accepted}, expected {expected}"
            );
        }
    }
}
