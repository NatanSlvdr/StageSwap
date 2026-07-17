use crate::{DetectionState, MonitorDescriptor, MonitorScore};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MonitorTrackerSettings {
    pub match_threshold: f64,
}
impl Default for MonitorTrackerSettings {
    fn default() -> Self {
        Self {
            match_threshold: 0.98,
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct MonitorTrackingResult {
    pub tracked: Option<MonitorDescriptor>,
    pub scan_state: DetectionState,
    pub best_similarity: f64,
    pub changed: bool,
    pub confirmation_pending: bool,
}

#[derive(Clone, Debug, Default)]
pub struct MonitorTracker {
    settings: MonitorTrackerSettings,
    tracked: Option<MonitorDescriptor>,
    pending_name: String,
    confirmations: u8,
}

impl MonitorTracker {
    pub fn new(settings: MonitorTrackerSettings) -> Self {
        Self {
            settings,
            ..Self::default()
        }
    }
    pub fn select(&mut self, monitor: MonitorDescriptor) {
        self.tracked = Some(monitor);
        self.pending_name.clear();
        self.confirmations = 0;
    }
    pub fn tracked(&self) -> Option<&MonitorDescriptor> {
        self.tracked.as_ref()
    }

    pub fn apply_scan(&mut self, scores: &[MonitorScore]) -> MonitorTrackingResult {
        let best = scores
            .iter()
            .filter(|score| score.capture_valid && score.similarity.is_finite())
            .max_by(|left, right| left.similarity.total_cmp(&right.similarity));
        let Some(best) = best.filter(|score| score.similarity >= self.settings.match_threshold)
        else {
            self.pending_name.clear();
            self.confirmations = 0;
            return MonitorTrackingResult {
                tracked: self.tracked.clone(),
                scan_state: DetectionState::ReferenceMissing,
                ..MonitorTrackingResult::default()
            };
        };
        if self
            .tracked
            .as_ref()
            .is_some_and(|monitor| monitor.display_name == best.monitor.display_name)
        {
            self.pending_name.clear();
            self.confirmations = 0;
            return MonitorTrackingResult {
                tracked: self.tracked.clone(),
                scan_state: DetectionState::Matching,
                best_similarity: best.similarity,
                ..MonitorTrackingResult::default()
            };
        }
        if self.pending_name == best.monitor.display_name {
            self.confirmations += 1;
        } else {
            self.pending_name.clone_from(&best.monitor.display_name);
            self.confirmations = 1;
        }
        if self.confirmations < 2 {
            return MonitorTrackingResult {
                tracked: self.tracked.clone(),
                scan_state: DetectionState::Matching,
                best_similarity: best.similarity,
                confirmation_pending: true,
                ..MonitorTrackingResult::default()
            };
        }
        self.tracked = Some(best.monitor.clone());
        self.pending_name.clear();
        self.confirmations = 0;
        MonitorTrackingResult {
            tracked: self.tracked.clone(),
            scan_state: DetectionState::Matching,
            best_similarity: best.similarity,
            changed: true,
            ..MonitorTrackingResult::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn monitor(name: &str) -> MonitorDescriptor {
        MonitorDescriptor {
            display_name: name.into(),
            ..MonitorDescriptor::default()
        }
    }
    #[test]
    fn requires_two_identical_winning_scans() {
        let mut tracker = MonitorTracker::new(MonitorTrackerSettings::default());
        tracker.select(monitor("one"));
        let scores = [MonitorScore {
            monitor: monitor("two"),
            similarity: 0.99,
            capture_valid: true,
        }];
        assert!(tracker.apply_scan(&scores).confirmation_pending);
        let confirmed = tracker.apply_scan(&scores);
        assert!(confirmed.changed);
        assert_eq!(confirmed.tracked.unwrap().display_name, "two");
    }
}
