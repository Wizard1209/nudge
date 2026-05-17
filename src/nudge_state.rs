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
    /// User briefly looked away (clicked outside the card or focused another
    /// window). Per spec §4: hide popup, restart timer, but keep `doing` /
    /// `bullshit` text so the next open resumes where the user left off.
    SwitchAway,
}

/// Decides whether the timer should be reset after a popup interaction.
/// Manual dismiss (tray-triggered Esc) is the only case that keeps the
/// existing timer running — the user opened the popup ad-hoc and changed
/// their mind. Submit, Dismiss-from-timer, and any SwitchAway all reset.
pub fn should_reset_timer(source: TriggerSource, action: Action) -> bool {
    !matches!((source, action), (TriggerSource::Manual, Action::Dismiss))
}

/// Error returned by `parse_interval` when the user typed a number that is
/// invalid per spec (`next_interval_minutes` must be finite and > 0).
#[derive(Debug, Clone, PartialEq)]
pub struct IntervalError {
    pub input: String,
}

impl std::fmt::Display for IntervalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Interval must be a positive number of minutes (got \"{}\")", self.input)
    }
}

/// Parse the minutes field into a Duration.
///
/// - Empty / whitespace / unparseable text → default of 10 minutes (user
///   didn't provide an explicit number, so we fall back silently).
/// - Parsed number ≤ 0 or non-finite → `Err(IntervalError)` — the user
///   explicitly typed something invalid and should be told, not silently
///   clamped (journal would otherwise record a number that doesn't match
///   what the user intended).
pub fn parse_interval(text: &str) -> Result<Duration, IntervalError> {
    let trimmed = text.trim();
    let Ok(minutes) = trimmed.parse::<f64>() else {
        return Ok(Duration::from_secs(10 * 60));
    };
    if !minutes.is_finite() || minutes <= 0.0 {
        return Err(IntervalError {
            input: trimmed.to_string(),
        });
    }
    Ok(Duration::from_secs_f64(minutes * 60.0))
}

/// Tray tooltip text for the time remaining until the next nudge.
/// Spec §5: `~N min` rounded UP to the next whole minute; `now` once the
/// timer has expired (showing `~0 min` would read as a bug rather than
/// "popup is about to appear").
pub fn tooltip_for_remaining(d: Duration) -> String {
    let secs = d.as_secs();
    if secs == 0 {
        return "now".to_string();
    }
    let mins = (secs + 59) / 60;
    format!("~{} min", mins)
}

/// Minute number that `tooltip_for_remaining` would render for this
/// duration. The tray loop uses this to dedupe `set_tooltip` calls so the
/// tooltip is only refreshed when the displayed number changes (spec §5:
/// "обновляется раз в минуту"). The "now" state maps to 0.
pub fn displayed_minutes(d: Duration) -> u64 {
    let secs = d.as_secs();
    if secs == 0 {
        0
    } else {
        (secs + 59) / 60
    }
}

/// Labels for the tray context menu, in order. Spec §5: "Show Nudge", "Quit".
pub const TRAY_MENU_LABELS: [&str; 2] = ["Show Nudge", "Quit"];

/// How often `update()` should re-run. `None` when the popup is hidden:
/// tray clicks and the timer-expiry thread wake the event loop via
/// `ShowWindow`, so no periodic wakeup is required. Returning `Some` in
/// the hidden state is expensive because the transparent always-on-top
/// window composites through DWM on every swap.
pub fn repaint_interval(popup_visible: bool) -> Option<Duration> {
    if popup_visible {
        Some(Duration::from_secs(1))
    } else {
        None
    }
}

/// Screen position for the spotlight window: horizontally centered; the top
/// of the window sits at 25% of screen height (spec §1 — верхняя треть, на
/// уровне естественного взгляда). Window height is irrelevant to the y anchor.
pub fn window_position(screen: (u32, u32), window: (u32, u32)) -> (i32, i32) {
    let (sw, sh) = (screen.0 as i32, screen.1 as i32);
    let (ww, _wh) = (window.0 as i32, window.1 as i32);
    let x = (sw - ww) / 2;
    let y = sh * 25 / 100;
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
    fn switch_away_always_resets() {
        assert!(should_reset_timer(TriggerSource::Timer, Action::SwitchAway));
        assert!(should_reset_timer(TriggerSource::Manual, Action::SwitchAway));
    }

    #[test]
    fn parse_interval_valid() {
        assert_eq!(parse_interval("15").unwrap(), Duration::from_secs(15 * 60));
    }

    #[test]
    fn parse_interval_gibberish_defaults_to_10() {
        assert_eq!(parse_interval("abc").unwrap(), Duration::from_secs(10 * 60));
    }

    #[test]
    fn parse_interval_empty_defaults_to_10() {
        assert_eq!(parse_interval("").unwrap(), Duration::from_secs(10 * 60));
        assert_eq!(parse_interval("   ").unwrap(), Duration::from_secs(10 * 60));
    }

    #[test]
    fn parse_interval_whitespace() {
        assert_eq!(parse_interval("  5  ").unwrap(), Duration::from_secs(5 * 60));
    }

    #[test]
    fn parse_interval_zero_is_error() {
        assert!(parse_interval("0").is_err());
    }

    #[test]
    fn parse_interval_float() {
        assert_eq!(parse_interval("0.1").unwrap(), Duration::from_secs(6));
    }

    #[test]
    fn parse_interval_negative_is_error() {
        assert!(parse_interval("-5").is_err());
        let err = parse_interval("-5").unwrap_err();
        assert_eq!(err.input, "-5");
    }

    #[test]
    fn parse_interval_nan_is_error() {
        assert!(parse_interval("NaN").is_err());
    }

    #[test]
    fn tooltip_rounds_up_to_next_minute() {
        assert_eq!(tooltip_for_remaining(Duration::from_secs(30)), "~1 min");
        assert_eq!(tooltip_for_remaining(Duration::from_secs(60)), "~1 min");
        assert_eq!(tooltip_for_remaining(Duration::from_secs(61)), "~2 min");
        assert_eq!(tooltip_for_remaining(Duration::from_secs(9 * 60)), "~9 min");
        assert_eq!(tooltip_for_remaining(Duration::from_secs(9 * 60 + 1)), "~10 min");
        assert_eq!(tooltip_for_remaining(Duration::from_secs(30 * 60)), "~30 min");
    }

    #[test]
    fn tooltip_zero_remaining_says_now() {
        // Past expiry — the popup is about to appear, "0 min" reads as a bug.
        assert_eq!(tooltip_for_remaining(Duration::from_secs(0)), "now");
    }

    #[test]
    fn displayed_minutes_matches_tooltip_count() {
        // The tray loop dedupes set_tooltip calls by this value. It must
        // change exactly when the rendered string's minute number changes,
        // and collapse the "now" state to 0.
        assert_eq!(displayed_minutes(Duration::from_secs(0)), 0);
        assert_eq!(displayed_minutes(Duration::from_secs(1)), 1);
        assert_eq!(displayed_minutes(Duration::from_secs(60)), 1);
        assert_eq!(displayed_minutes(Duration::from_secs(61)), 2);
        assert_eq!(displayed_minutes(Duration::from_secs(30 * 60)), 30);
    }

    #[test]
    fn tray_menu_labels_match_spec() {
        assert_eq!(TRAY_MENU_LABELS, ["Show Nudge", "Quit"]);
    }

    #[test]
    fn repaint_interval_visible_ticks_every_second() {
        assert_eq!(repaint_interval(true), Some(Duration::from_secs(1)));
    }

    #[test]
    fn repaint_interval_hidden_is_none() {
        // When popup is hidden, no periodic repaint should be scheduled.
        // Tray clicks and the timer-expiry thread wake update() via
        // ShowWindow — a 1 Hz wakeup would just burn CPU redrawing a
        // transparent layered window through DWM.
        assert_eq!(repaint_interval(false), None);
    }

    #[test]
    fn window_position_top_at_25pct_full_hd() {
        // 1920×1080, window 520×320 → x centered = (1920-520)/2 = 700;
        // top of window at 25% of 1080 = 270.
        assert_eq!(window_position((1920, 1080), (520, 320)), (700, 270));
    }

    #[test]
    fn window_position_top_at_25pct_small_screen() {
        // 1366×768, window 480×280 → x centered = (1366-480)/2 = 443;
        // top of window at 25% of 768 = 192.
        assert_eq!(window_position((1366, 768), (480, 280)), (443, 192));
    }
}
