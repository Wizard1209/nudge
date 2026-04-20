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
/// Accepts floats (e.g. "0.1" = 6 seconds). Values ≤ 0 become 1 second.
pub fn parse_interval(text: &str) -> Duration {
    let minutes: f64 = text.trim().parse().unwrap_or(10.0);
    if minutes <= 0.0 {
        Duration::from_secs(1)
    } else {
        Duration::from_secs_f64(minutes * 60.0)
    }
}

/// Format a tray tooltip string rounded UP to the next minute.
/// Spec: tray tooltip reads "Nudge in N min", updated no more often than once a minute.
pub fn tooltip_for_remaining(d: Duration) -> String {
    let secs = d.as_secs();
    let mins = (secs + 59) / 60;
    format!("Nudge in {} min", mins)
}

/// Labels for the tray context menu, in order. Spec §5: "Show Nudge", "Quit".
pub const TRAY_MENU_LABELS: [&str; 2] = ["Show Nudge", "Quit"];

/// Screen position for the spotlight window: horizontally centered, vertically
/// placed so the window's center sits at 40% of screen height (spec §1).
pub fn window_position(screen: (u32, u32), window: (u32, u32)) -> (i32, i32) {
    let (sw, sh) = (screen.0 as i32, screen.1 as i32);
    let (ww, wh) = (window.0 as i32, window.1 as i32);
    let x = (sw - ww) / 2;
    let y = sh * 40 / 100 - wh / 2;
    (x, y)
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

    #[test]
    fn parse_interval_float() {
        assert_eq!(parse_interval("0.1"), Duration::from_secs(6));
    }

    #[test]
    fn parse_interval_negative_becomes_1_second() {
        assert_eq!(parse_interval("-5"), Duration::from_secs(1));
    }

    #[test]
    fn tooltip_rounds_up_to_next_minute() {
        assert_eq!(tooltip_for_remaining(Duration::from_secs(30)), "Nudge in 1 min");
        assert_eq!(tooltip_for_remaining(Duration::from_secs(60)), "Nudge in 1 min");
        assert_eq!(tooltip_for_remaining(Duration::from_secs(61)), "Nudge in 2 min");
        assert_eq!(tooltip_for_remaining(Duration::from_secs(9 * 60)), "Nudge in 9 min");
        assert_eq!(tooltip_for_remaining(Duration::from_secs(9 * 60 + 1)), "Nudge in 10 min");
    }

    #[test]
    fn tooltip_zero_remaining_is_zero_min() {
        assert_eq!(tooltip_for_remaining(Duration::from_secs(0)), "Nudge in 0 min");
    }

    #[test]
    fn tray_menu_labels_match_spec() {
        assert_eq!(TRAY_MENU_LABELS, ["Show Nudge", "Quit"]);
    }

    #[test]
    fn window_position_horizontally_centered_40pct_vertical() {
        assert_eq!(window_position((1920, 1080), (520, 320)), (700, 272));
    }

    #[test]
    fn window_position_small_screen() {
        assert_eq!(window_position((1366, 768), (480, 280)), (443, 167));
    }
}
