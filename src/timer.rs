use std::time::Duration;
use web_time::Instant;

pub struct Timer {
    deadline: Instant,
}

impl Timer {
    pub fn new(duration: Duration) -> Self {
        Self {
            deadline: Instant::now() + duration,
        }
    }

    pub fn is_expired(&self) -> bool {
        Instant::now() >= self.deadline
    }

    pub fn reset(&mut self, duration: Duration) {
        self.deadline = Instant::now() + duration;
    }

    pub fn remaining(&self) -> Duration {
        self.deadline.saturating_duration_since(Instant::now())
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
}
