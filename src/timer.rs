use std::time::Duration;
use web_time::Instant;

pub struct Timer {
    // None = frozen: timer never expires until `reset()` is called.
    // Spec §4: on first launch the popup is up, and the timer only starts
    // ticking after the first close (Submit / Esc / Switch).
    deadline: Option<Instant>,
}

impl Timer {
    #[cfg(test)]
    pub fn new(duration: Duration) -> Self {
        Self {
            deadline: Some(Instant::now() + duration),
        }
    }

    pub fn frozen() -> Self {
        Self { deadline: None }
    }

    #[cfg(any(test, target_arch = "wasm32"))]
    pub fn is_expired(&self) -> bool {
        match self.deadline {
            Some(d) => Instant::now() >= d,
            None => false,
        }
    }

    pub fn reset(&mut self, duration: Duration) {
        self.deadline = Some(Instant::now() + duration);
    }

    #[cfg(any(test, target_arch = "wasm32"))]
    pub fn remaining(&self) -> Duration {
        match self.deadline {
            Some(d) => d.saturating_duration_since(Instant::now()),
            None => Duration::ZERO,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_timer_is_not_expired() {
        let timer = Timer::new(Duration::from_secs(60));
        assert!(!timer.is_expired());
    }

    #[test]
    fn timer_expires_after_deadline() {
        let timer = Timer::new(Duration::from_millis(10));
        std::thread::sleep(Duration::from_millis(20));
        assert!(timer.is_expired());
    }

    #[test]
    fn reset_extends_deadline() {
        let mut timer = Timer::new(Duration::from_millis(10));
        std::thread::sleep(Duration::from_millis(20));
        assert!(timer.is_expired());

        timer.reset(Duration::from_secs(60));
        assert!(!timer.is_expired());
    }

    #[test]
    fn remaining_decreases_over_time() {
        let timer = Timer::new(Duration::from_secs(10));
        let r1 = timer.remaining();
        std::thread::sleep(Duration::from_millis(50));
        let r2 = timer.remaining();
        assert!(r2 < r1);
    }

    #[test]
    fn remaining_is_zero_when_expired() {
        let timer = Timer::new(Duration::from_millis(10));
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(timer.remaining(), Duration::ZERO);
    }

    #[test]
    fn frozen_timer_is_never_expired() {
        let timer = Timer::frozen();
        assert!(!timer.is_expired());
    }

    #[test]
    fn frozen_timer_does_not_expire_with_time() {
        let timer = Timer::frozen();
        std::thread::sleep(Duration::from_millis(20));
        assert!(!timer.is_expired());
    }

    #[test]
    fn reset_unfreezes_timer() {
        let mut timer = Timer::frozen();
        timer.reset(Duration::from_millis(10));
        std::thread::sleep(Duration::from_millis(20));
        assert!(timer.is_expired());
    }
}
