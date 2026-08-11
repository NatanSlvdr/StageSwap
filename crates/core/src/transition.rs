use crate::Source;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TransitionState {
    pub logical_source: Source,
    pub target: Source,
    pub screen_mix: f64,
    pub active: bool,
    pub reversed: bool,
    pub remaining: Duration,
}

impl Default for TransitionState {
    fn default() -> Self {
        Self {
            logical_source: Source::Camera,
            target: Source::Camera,
            screen_mix: 0.0,
            active: false,
            reversed: false,
            remaining: Duration::ZERO,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TransitionController {
    duration: Duration,
    state: TransitionState,
    last_update: Option<Instant>,
}

impl TransitionController {
    pub fn new(duration: Duration) -> Self {
        Self {
            duration,
            state: TransitionState::default(),
            last_update: None,
        }
    }
    pub fn state(&self) -> TransitionState {
        self.state
    }

    fn advance(&mut self, now: Instant) {
        let Some(previous) = self.last_update.replace(now) else {
            return;
        };
        if !self.state.active {
            return;
        }
        let delta =
            now.saturating_duration_since(previous).as_secs_f64() / self.duration.as_secs_f64();
        let target_mix = if self.state.target == Source::Screen {
            1.0
        } else {
            0.0
        };
        if target_mix > self.state.screen_mix {
            self.state.screen_mix = (self.state.screen_mix + delta).min(target_mix);
        } else {
            self.state.screen_mix = (self.state.screen_mix - delta).max(target_mix);
        }
        self.state.active = (self.state.screen_mix - target_mix).abs() > f64::EPSILON;
        if !self.state.active {
            self.state.logical_source = self.state.target;
            self.state.remaining = Duration::ZERO;
        } else {
            self.state.remaining = self
                .duration
                .mul_f64((self.state.screen_mix - target_mix).abs());
        }
    }

    pub fn request(&mut self, target: Source, now: Instant) -> TransitionState {
        self.advance(now);
        if target == Source::Placeholder {
            self.state = TransitionState {
                logical_source: Source::Placeholder,
                target: Source::Placeholder,
                ..TransitionState::default()
            };
            return self.state;
        }
        let effective = target;
        self.state.reversed = self.state.active && effective != self.state.target;
        self.state.target = effective;
        let target_mix: f64 = if effective == Source::Screen {
            1.0
        } else {
            0.0
        };
        self.state.active = (self.state.screen_mix - target_mix).abs() > f64::EPSILON;
        self.state.remaining = self
            .duration
            .mul_f64((self.state.screen_mix - target_mix).abs());
        self.state
    }

    pub fn tick(&mut self, now: Instant) -> TransitionState {
        self.advance(now);
        self.state
    }
}

impl Default for TransitionController {
    fn default() -> Self {
        Self::new(Duration::from_millis(500))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contract_fade_reverses_without_a_jump() {
        let start = Instant::now();
        let mut transition = TransitionController::default();
        transition.tick(start);
        transition.request(Source::Screen, start);
        assert!(
            (transition
                .tick(start + Duration::from_millis(300))
                .screen_mix
                - 0.6)
                .abs()
                < 0.001
        );
        let reversed = transition.request(Source::Camera, start + Duration::from_millis(300));
        assert!(reversed.reversed);
        assert!((reversed.screen_mix - 0.6).abs() < 0.001);
        assert!(
            (transition
                .tick(start + Duration::from_millis(450))
                .screen_mix
                - 0.3)
                .abs()
                < 0.001
        );
    }
}
