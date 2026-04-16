use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TriggerSource {
    Timer,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Submit,
    Dismiss,
}

/// Decides whether the timer should be reset after a popup interaction.
/// Manual dismiss (tray-triggered Esc) keeps the existing timer running.
pub fn should_reset_timer(source: TriggerSource, action: Action) -> bool {
    !matches!((source, action), (TriggerSource::Manual, Action::Dismiss))
}

/// Parse the minutes field into a Duration, defaulting to 10 min.
/// Values ≤ 0 become 1 second (useful for testing).
pub fn parse_interval(text: &str) -> Duration {
    let minutes: u64 = text.trim().parse().unwrap_or(10);
    if minutes == 0 {
        Duration::from_secs(1)
    } else {
        Duration::from_secs(minutes * 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timer_submit_resets() {
        assert!(should_reset_timer(TriggerSource::Timer, Action::Submit));
    }

    #[test]
    fn timer_dismiss_resets() {
        assert!(should_reset_timer(TriggerSource::Timer, Action::Dismiss));
    }

    #[test]
    fn manual_submit_resets() {
        assert!(should_reset_timer(TriggerSource::Manual, Action::Submit));
    }

    #[test]
    fn manual_dismiss_does_not_reset() {
        assert!(!should_reset_timer(TriggerSource::Manual, Action::Dismiss));
    }

    #[test]
    fn parse_interval_valid() {
        assert_eq!(parse_interval("15"), Duration::from_secs(15 * 60));
    }

    #[test]
    fn parse_interval_invalid_defaults_to_10() {
        assert_eq!(parse_interval("abc"), Duration::from_secs(10 * 60));
    }

    #[test]
    fn parse_interval_whitespace() {
        assert_eq!(parse_interval("  5  "), Duration::from_secs(5 * 60));
    }

    #[test]
    fn parse_interval_zero_becomes_1_second() {
        assert_eq!(parse_interval("0"), Duration::from_secs(1));
    }
}
