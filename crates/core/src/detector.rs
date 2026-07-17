use crate::DetectionState;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct DetectorSettings {
    pub threshold: f64,
    pub matches_required: u32,
    pub mismatches_required: u32,
}

impl Default for DetectorSettings {
    fn default() -> Self {
        Self {
            threshold: 0.98,
            matches_required: 5,
            mismatches_required: 3,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct DebouncedDetector {
    settings: DetectorSettings,
    state: DetectionState,
    matches: u32,
    mismatches: u32,
    similarity: f64,
}

impl DebouncedDetector {
    pub fn new(settings: DetectorSettings) -> Self {
        Self {
            settings,
            ..Self::default()
        }
    }
    pub fn state(&self) -> DetectionState {
        self.state
    }
    pub fn similarity(&self) -> f64 {
        self.similarity
    }
    pub fn counters(&self) -> (u32, u32) {
        (self.matches, self.mismatches)
    }
    pub fn reset(&mut self) {
        self.state = DetectionState::Unknown;
        self.matches = 0;
        self.mismatches = 0;
    }

    pub fn update(&mut self, similarity: f64, capture_valid: bool) -> DetectionState {
        self.similarity = similarity;
        if !capture_valid || !similarity.is_finite() {
            self.matches = 0;
            self.mismatches = 0;
            self.state = DetectionState::ReferenceMissing;
        } else if similarity >= self.settings.threshold {
            self.matches = self.matches.saturating_add(1);
            self.mismatches = 0;
            if self.matches >= self.settings.matches_required {
                self.state = DetectionState::Matching;
            }
        } else {
            self.mismatches = self.mismatches.saturating_add(1);
            self.matches = 0;
            if self.mismatches >= self.settings.mismatches_required {
                self.state = DetectionState::NotMatching;
            }
        }
        self.state
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn applies_five_three_debounce() {
        let mut detector = DebouncedDetector::new(DetectorSettings::default());
        for _ in 0..4 {
            assert_eq!(detector.update(0.99, true), DetectionState::Unknown);
        }
        assert_eq!(detector.update(0.99, true), DetectionState::Matching);
        for _ in 0..2 {
            assert_eq!(detector.update(0.1, true), DetectionState::Matching);
        }
        assert_eq!(detector.update(0.1, true), DetectionState::NotMatching);
    }
}
