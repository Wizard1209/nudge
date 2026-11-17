use std::time::Duration;

use crate::config::{self, IntervalError};

fn default_interval() -> Duration {
    Duration::from_secs_f64(config::DEFAULT_INTERVAL_MINUTES * 60.0)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TriggerSource {
    Timer,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Action {
    Submit,
    Dismiss,
    SwitchAway,
}

#[derive(Debug, Clone, PartialEq)]
pub struct CloseOutcome {
    pub journal_minutes: Option<f64>,
    pub reset_timer: Option<Duration>,
    pub clear_form: bool,
}

pub fn decide_close(
    source: TriggerSource,
    action: Action,
    doing: &str,
    bullshit: &str,
    minutes_text: &str,
) -> Result<CloseOutcome, IntervalError> {
    let interval = match parse_interval(minutes_text) {
        Ok(d) => d,
        Err(e) => {
            if action == Action::Submit {
                return Err(e);
            }
            default_interval()
        }
    };

    let reset = matches!(source, TriggerSource::Timer) || matches!(action, Action::Submit);

    let write_journal =
        action == Action::Submit && !(doing.trim().is_empty() && bullshit.trim().is_empty());

    Ok(CloseOutcome {
        journal_minutes: write_journal.then(|| interval.as_secs_f64() / 60.0),
        reset_timer: reset.then_some(interval),
        clear_form: matches!(action, Action::Submit),
    })
}

pub fn launch_interval(minutes_text: &str) -> Duration {
    parse_interval(minutes_text).unwrap_or_else(|_| default_interval())
}

pub fn parse_interval(text: &str) -> Result<Duration, IntervalError> {
    let trimmed = text.trim();
    let Ok(minutes) = trimmed.parse::<f64>() else {
        return Ok(default_interval());
    };
    if !config::valid_interval_minutes(minutes) {
        return Err(IntervalError {
            input: trimmed.to_string(),
        });
    }
    Ok(Duration::from_secs_f64(minutes * 60.0))
}

pub fn displayed_minutes(d: Duration) -> u64 {
    let secs = d.as_secs();
    if secs == 0 { 0 } else { secs.div_ceil(60) }
}

pub fn tooltip_for_remaining(d: Duration) -> String {
    match displayed_minutes(d) {
        0 => "now".to_string(),
        mins => format!("~{} min", mins),
    }
}

pub const TRAY_MENU_LABELS: [&str; 3] = ["Show Nudge", "Settings", "Quit"];

pub fn repaint_interval(popup_visible: bool) -> Option<Duration> {
    if popup_visible {
        Some(Duration::from_secs(1))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn five_min() -> Duration {
        Duration::from_secs(5 * 60)
    }

    #[test]
    fn timer_submit_with_content_writes_resets_clears() {
        let o = decide_close(TriggerSource::Timer, Action::Submit, "doing", "", "5").unwrap();
        assert_eq!(
            o,
            CloseOutcome {
                journal_minutes: Some(5.0),
                reset_timer: Some(five_min()),
                clear_form: true,
            }
        );
    }

    #[test]
    fn timer_dismiss_resets_but_writes_nothing_and_keeps_form() {
        let o = decide_close(TriggerSource::Timer, Action::Dismiss, "doing", "x", "5").unwrap();
        assert_eq!(
            o.reset_timer,
            Some(five_min()),
            "timer-fired Esc must reset"
        );
        assert_eq!(o.journal_minutes, None);
        assert!(!o.clear_form);
    }

    #[test]
    fn timer_switch_away_also_resets() {
        let o = decide_close(TriggerSource::Timer, Action::SwitchAway, "", "", "5").unwrap();
        assert_eq!(o.reset_timer, Some(five_min()));
        assert_eq!(o.journal_minutes, None);
        assert!(!o.clear_form);
    }

    #[test]
    fn manual_dismiss_leaves_timer_alone() {
        let o = decide_close(TriggerSource::Manual, Action::Dismiss, "doing", "x", "5").unwrap();
        assert_eq!(o.reset_timer, None, "manual Esc must not reschedule");
        assert_eq!(o.journal_minutes, None);
        assert!(!o.clear_form);
    }

    #[test]
    fn manual_switch_away_leaves_timer_alone() {
        let o = decide_close(TriggerSource::Manual, Action::SwitchAway, "doing", "", "5").unwrap();
        assert_eq!(o.reset_timer, None);
        assert_eq!(o.journal_minutes, None);
        assert!(!o.clear_form);
    }

    #[test]
    fn manual_submit_resets_even_though_manual() {
        let o = decide_close(TriggerSource::Manual, Action::Submit, "doing", "", "5").unwrap();
        assert_eq!(o.reset_timer, Some(five_min()));
        assert!(o.journal_minutes.is_some());
        assert!(o.clear_form);
    }

    #[test]
    fn submit_with_empty_fields_is_change_interval_without_journaling() {
        for (doing, bullshit) in [("", ""), ("   ", "\t\n")] {
            let o =
                decide_close(TriggerSource::Manual, Action::Submit, doing, bullshit, "5").unwrap();
            assert_eq!(o.journal_minutes, None, "empty Submit must not write");
            assert_eq!(o.reset_timer, Some(five_min()), "but it still resets");
            assert!(o.clear_form);
        }
    }

    #[test]
    fn submit_writes_when_either_field_nonempty() {
        for (doing, bullshit) in [("x", ""), ("", "y"), ("x", "y")] {
            let o =
                decide_close(TriggerSource::Manual, Action::Submit, doing, bullshit, "5").unwrap();
            assert!(
                o.journal_minutes.is_some(),
                "Submit with content must write"
            );
        }
    }

    #[test]
    fn submit_with_invalid_interval_is_a_validation_error() {
        for bad in ["0", "-5", "NaN"] {
            let err =
                decide_close(TriggerSource::Manual, Action::Submit, "doing", "", bad).unwrap_err();
            assert_eq!(err.input, bad);
        }
    }

    #[test]
    fn dismiss_with_invalid_interval_falls_back_and_still_closes() {
        let o = decide_close(TriggerSource::Timer, Action::Dismiss, "doing", "", "-5").unwrap();
        assert_eq!(o.reset_timer, Some(Duration::from_secs(10 * 60)));
        assert_eq!(o.journal_minutes, None);
    }

    #[test]
    fn an_interval_too_large_to_arm_is_refused_on_submit_and_falls_back_on_leave() {
        let err =
            decide_close(TriggerSource::Timer, Action::Submit, "doing", "", "1e20").unwrap_err();
        assert_eq!(err.input, "1e20");
        for action in [Action::SwitchAway, Action::Dismiss] {
            let o = decide_close(TriggerSource::Timer, action, "doing", "", "1e20").unwrap();
            assert_eq!(
                o.reset_timer,
                Some(Duration::from_secs(10 * 60)),
                "{action:?} closes on the 10-minute default"
            );
            assert_eq!(o.journal_minutes, None);
        }
    }

    #[test]
    fn manual_dismiss_with_invalid_interval_does_not_reset() {
        let o = decide_close(
            TriggerSource::Manual,
            Action::Dismiss,
            "doing",
            "",
            "garbage",
        )
        .unwrap();
        assert_eq!(o.reset_timer, None);
    }

    #[test]
    fn launch_arms_the_minutes_field() {
        assert_eq!(launch_interval("7"), Duration::from_secs(7 * 60));
        assert_eq!(launch_interval("0.05"), Duration::from_secs(3));
    }

    #[test]
    fn launch_never_refuses_a_field_and_arms_the_default_instead() {
        for text in ["", "abc", "0", "-5", "NaN", "1e20"] {
            assert_eq!(launch_interval(text), Duration::from_secs(600), "{text:?}");
        }
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
        assert_eq!(
            parse_interval("  5  ").unwrap(),
            Duration::from_secs(5 * 60)
        );
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
        assert_eq!(
            tooltip_for_remaining(Duration::from_secs(9 * 60 + 1)),
            "~10 min"
        );
        assert_eq!(
            tooltip_for_remaining(Duration::from_secs(30 * 60)),
            "~30 min"
        );
    }

    #[test]
    fn tooltip_zero_remaining_says_now() {
        assert_eq!(tooltip_for_remaining(Duration::from_secs(0)), "now");
    }

    #[test]
    fn displayed_minutes_matches_tooltip_count() {
        assert_eq!(displayed_minutes(Duration::from_secs(0)), 0);
        assert_eq!(displayed_minutes(Duration::from_secs(1)), 1);
        assert_eq!(displayed_minutes(Duration::from_secs(60)), 1);
        assert_eq!(displayed_minutes(Duration::from_secs(61)), 2);
        assert_eq!(displayed_minutes(Duration::from_secs(30 * 60)), 30);
    }

    #[test]
    fn tray_menu_labels_match_spec() {
        assert_eq!(TRAY_MENU_LABELS, ["Show Nudge", "Settings", "Quit"]);
    }

    #[test]
    fn repaint_interval_visible_ticks_every_second() {
        assert_eq!(repaint_interval(true), Some(Duration::from_secs(1)));
    }

    #[test]
    fn repaint_interval_hidden_is_none() {
        assert_eq!(repaint_interval(false), None);
    }
}
