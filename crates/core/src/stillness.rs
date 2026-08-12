use crate::GrayImage;
use std::time::{Duration, Instant};

#[derive(Clone, Debug, Default)]
pub struct StillImageDetector {
    baseline: Option<GrayImage>,
    still_since: Option<Instant>,
    active: bool,
}

impl StillImageDetector {
    pub fn active(&self) -> bool {
        self.active
    }

    pub fn reset(&mut self) {
        self.baseline = None;
        self.still_since = None;
        self.active = false;
    }

    pub fn update(
        &mut self,
        candidate: Option<&GrayImage>,
        eligible: bool,
        delay: Duration,
        now: Instant,
    ) -> bool {
        let Some(candidate) = candidate.filter(|_| eligible) else {
            self.reset();
            return false;
        };

        if self.baseline.as_ref() != Some(candidate) {
            self.baseline = Some(candidate.clone());
            self.still_since = Some(now);
            self.active = false;
            return false;
        }

        let since = self.still_since.get_or_insert(now);
        self.active = now.saturating_duration_since(*since) >= delay;
        self.active
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Size;

    fn image(value: u8) -> GrayImage {
        GrayImage::new(Size::new(2, 2), vec![value; 4]).unwrap()
    }

    #[test]
    fn contract_requires_one_continuous_exactly_still_interval() {
        let start = Instant::now();
        let delay = Duration::from_secs(45);
        let first = image(10);
        let changed = image(11);
        let mut detector = StillImageDetector::default();

        assert!(!detector.update(Some(&first), true, delay, start));
        assert!(!detector.update(Some(&first), true, delay, start + Duration::from_secs(44)));
        assert!(detector.update(Some(&first), true, delay, start + Duration::from_secs(45)));

        assert!(!detector.update(Some(&changed), true, delay, start + Duration::from_secs(46)));
        assert!(!detector.update(Some(&first), true, delay, start + Duration::from_secs(47)));
        assert!(detector.update(Some(&first), true, delay, start + Duration::from_secs(92)));
    }

    #[test]
    fn contract_ineligible_or_missing_samples_reset_the_interval() {
        let start = Instant::now();
        let delay = Duration::from_secs(30);
        let frame = image(10);
        let mut detector = StillImageDetector::default();

        detector.update(Some(&frame), true, delay, start);
        assert!(!detector.update(Some(&frame), false, delay, start + Duration::from_secs(30)));
        assert!(!detector.active());
        assert!(!detector.update(None, true, delay, start + Duration::from_secs(31)));
        assert!(!detector.update(Some(&frame), true, delay, start + Duration::from_secs(60)));
    }
}
