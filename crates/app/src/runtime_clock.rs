use std::time::Instant;

pub trait RuntimeClock: Send + Sync + 'static {
    fn now(&self) -> Instant;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct SystemRuntimeClock;

impl RuntimeClock for SystemRuntimeClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct VirtualRuntimeClock {
    now: std::sync::Arc<std::sync::Mutex<Instant>>,
}

#[cfg(test)]
impl VirtualRuntimeClock {
    pub(crate) fn new(now: Instant) -> Self {
        Self {
            now: std::sync::Arc::new(std::sync::Mutex::new(now)),
        }
    }

    pub(crate) fn advance(&self, duration: std::time::Duration) {
        let mut now = self.now.lock().expect("virtual clock lock is not poisoned");
        *now += duration;
    }
}

#[cfg(test)]
impl RuntimeClock for VirtualRuntimeClock {
    fn now(&self) -> Instant {
        *self.now.lock().expect("virtual clock lock is not poisoned")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn flow_virtual_clock_advances_without_sleeping() {
        let start = Instant::now();
        let clock = VirtualRuntimeClock::new(start);
        clock.advance(Duration::from_secs(30));
        assert_eq!(clock.now(), start + Duration::from_secs(30));
    }
}
