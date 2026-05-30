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

/// The full plan for closing the popup, computed in one place from the spec
/// §4 truth table. `decide_close` returns this; the caller (`NudgeApp::
/// hide_popup`) is a thin executor that performs exactly the I/O the plan
/// dictates — write the journal iff `write_journal`, reset the timer iff
/// `reset_timer` is `Some`, clear the text fields iff `clear_form`.
///
/// Folding the four old `should_*` predicates + interval parsing into one
/// outcome makes spec §4 a single coherent decision instead of four
/// disconnected booleans an orchestrator had to re-assemble. The composed
/// outcome — not the isolated booleans — is what historically broke (e.g.
/// "timer-fired Esc must still reset or the popup refires"), so it is also
/// the right test surface.
#[derive(Debug, Clone, PartialEq)]
pub struct CloseOutcome {
    /// Append a journal entry for this close. Only Submit with at least one
    /// non-empty text field; the empty-Submit path is "change interval
    /// without journaling".
    pub write_journal: bool,
    /// Reset the timer to this interval, or `None` to leave it ticking.
    /// `Some` whenever the popup was timer-fired (its deadline is already in
    /// the past — not rescheduling would refire instantly) or the user hit
    /// Submit (they explicitly chose a new interval).
    pub reset_timer: Option<Duration>,
    /// Clear `doing` / `bullshit` after close. Only Submit; Esc and Switch
    /// preserve them so the next open resumes where the user left off.
    pub clear_form: bool,
}

/// Decide everything that happens when the popup closes, from the spec §4
/// truth table, in one place. `minutes_text` is the raw contents of the
/// interval field.
///
/// Returns `Err(IntervalError)` only on the one validation path the spec
/// defines: an explicit Submit carrying a parseably-invalid interval (e.g.
/// "-5" or "0"). In that case the caller must surface the error and keep the
/// popup open — there is no valid outcome to execute. Esc / Switch never
/// error: they fall back to the 10-minute default so the popup always
/// closes, even with garbage in the field.
pub fn decide_close(
    source: TriggerSource,
    action: Action,
    doing: &str,
    bullshit: &str,
    minutes_text: &str,
) -> Result<CloseOutcome, IntervalError> {
    // Parse the interval. A parse error means the user typed an explicit
    // non-positive / non-finite number. On Submit that is a validation error
    // (caller keeps the popup open). On Esc / Switch we silently fall back to
    // the default so the popup still closes.
    let interval = match parse_interval(minutes_text) {
        Ok(d) => d,
        Err(e) => {
            if action == Action::Submit {
                return Err(e);
            }
            Duration::from_secs(10 * 60)
        }
    };

    // Timer-fired popup deadline is already in the past, so ANY close must
    // reschedule; otherwise Submit (user chose a new interval) resets too.
    let reset = matches!(source, TriggerSource::Timer) || matches!(action, Action::Submit);

    // Submit writes only if at least one free-text field is non-empty.
    let write_journal =
        action == Action::Submit && !(doing.trim().is_empty() && bullshit.trim().is_empty());

    Ok(CloseOutcome {
        write_journal,
        reset_timer: reset.then_some(interval),
        // Only Submit clears the form; Esc and Switch preserve it.
        clear_form: matches!(action, Action::Submit),
    })
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

/// Remaining whole minutes, rounded UP. Zero seconds stays 0 (the "now"
/// state); any positive remainder rounds to the next whole minute. Shared
/// by the tooltip text and its dedup key so the two never disagree.
fn ceil_minutes(d: Duration) -> u64 {
    let secs = d.as_secs();
    if secs == 0 { 0 } else { (secs + 59) / 60 }
}

/// Tray tooltip text for the time remaining until the next nudge.
/// Spec §5: `~N min` rounded UP to the next whole minute; `now` once the
/// timer has expired (showing `~0 min` would read as a bug rather than
/// "popup is about to appear").
pub fn tooltip_for_remaining(d: Duration) -> String {
    match ceil_minutes(d) {
        0 => "now".to_string(),
        mins => format!("~{} min", mins),
    }
}

/// Minute number that `tooltip_for_remaining` would render for this
/// duration. The tray loop uses this to dedupe `set_tooltip` calls so the
/// tooltip is only refreshed when the displayed number changes (spec §5:
/// "обновляется раз в минуту"). The "now" state maps to 0.
pub fn displayed_minutes(d: Duration) -> u64 {
    ceil_minutes(d)
}

/// Labels for the tray context menu, in order. Spec §5: tray language is
/// English. "Settings" sits between "Show Nudge" and "Quit" — visit-then-bye.
pub const TRAY_MENU_LABELS: [&str; 3] = ["Show Nudge", "Settings", "Quit"];

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

    // === decide_close: the spec §4 close decision, as one composed outcome ===
    //
    // These assert the whole `CloseOutcome` per row of the §4 truth table —
    // the composition is what historically broke, not the isolated booleans
    // it's built from. `D` is a valid interval ("5" → 5 minutes) so the
    // reset field carries the parsed value, not a fallback.

    /// 5 minutes as a Duration — the interval `"5"` parses to.
    fn five_min() -> Duration {
        Duration::from_secs(5 * 60)
    }

    #[test]
    fn timer_submit_with_content_writes_resets_clears() {
        // Timer-fired Enter with text: write the journal, reset to the chosen
        // interval, clear the form.
        let o = decide_close(TriggerSource::Timer, Action::Submit, "doing", "", "5").unwrap();
        assert_eq!(
            o,
            CloseOutcome {
                write_journal: true,
                reset_timer: Some(five_min()),
                clear_form: true,
            }
        );
    }

    #[test]
    fn timer_dismiss_resets_but_writes_nothing_and_keeps_form() {
        // Structural rule (autopsy: nudge-electron 2026-04-20): a timer-fired
        // popup's deadline is already in the past, so Esc MUST reschedule or
        // the popup refires next frame. It still writes nothing and preserves
        // the form.
        let o = decide_close(TriggerSource::Timer, Action::Dismiss, "doing", "x", "5").unwrap();
        assert_eq!(o.reset_timer, Some(five_min()), "timer-fired Esc must reset");
        assert!(!o.write_journal);
        assert!(!o.clear_form);
    }

    #[test]
    fn timer_switch_away_also_resets() {
        // Same structural reason as the Dismiss case.
        let o = decide_close(TriggerSource::Timer, Action::SwitchAway, "", "", "5").unwrap();
        assert_eq!(o.reset_timer, Some(five_min()));
        assert!(!o.write_journal);
        assert!(!o.clear_form);
    }

    #[test]
    fn manual_dismiss_leaves_timer_alone() {
        // User opened the popup manually and changed their mind — the live
        // timer keeps counting (reset_timer None), nothing is written, the
        // form is preserved.
        let o = decide_close(TriggerSource::Manual, Action::Dismiss, "doing", "x", "5").unwrap();
        assert_eq!(o.reset_timer, None, "manual Esc must not reschedule");
        assert!(!o.write_journal);
        assert!(!o.clear_form);
    }

    #[test]
    fn manual_switch_away_leaves_timer_alone() {
        let o = decide_close(TriggerSource::Manual, Action::SwitchAway, "doing", "", "5").unwrap();
        assert_eq!(o.reset_timer, None);
        assert!(!o.write_journal);
        assert!(!o.clear_form);
    }

    #[test]
    fn manual_submit_resets_even_though_manual() {
        // Submit always resets, regardless of trigger source — the user
        // explicitly chose a new interval.
        let o = decide_close(TriggerSource::Manual, Action::Submit, "doing", "", "5").unwrap();
        assert_eq!(o.reset_timer, Some(five_min()));
        assert!(o.write_journal);
        assert!(o.clear_form);
    }

    #[test]
    fn submit_with_empty_fields_is_change_interval_without_journaling() {
        // The dedicated "обновить интервал, ничего не записывая" path: Enter
        // on empty fields resets the timer but writes nothing. Whitespace-only
        // counts as empty.
        for (doing, bullshit) in [("", ""), ("   ", "\t\n")] {
            let o =
                decide_close(TriggerSource::Manual, Action::Submit, doing, bullshit, "5").unwrap();
            assert!(!o.write_journal, "empty Submit must not write");
            assert_eq!(o.reset_timer, Some(five_min()), "but it still resets");
            assert!(o.clear_form);
        }
    }

    #[test]
    fn submit_writes_when_either_field_nonempty() {
        for (doing, bullshit) in [("x", ""), ("", "y"), ("x", "y")] {
            let o =
                decide_close(TriggerSource::Manual, Action::Submit, doing, bullshit, "5").unwrap();
            assert!(o.write_journal, "Submit with content must write");
        }
    }

    #[test]
    fn submit_with_invalid_interval_is_a_validation_error() {
        // An explicit Submit carrying a parseably-invalid interval is the one
        // path that yields no outcome — the caller keeps the popup open and
        // shows the error.
        for bad in ["0", "-5", "NaN"] {
            let err = decide_close(TriggerSource::Manual, Action::Submit, "doing", "", bad)
                .unwrap_err();
            assert_eq!(err.input, bad);
        }
    }

    #[test]
    fn dismiss_with_invalid_interval_falls_back_and_still_closes() {
        // Esc / Switch never error: a garbage interval falls back to the
        // 10-minute default so the popup always closes. Combined with a
        // timer-fired source that means it resets to 10 minutes.
        let o = decide_close(TriggerSource::Timer, Action::Dismiss, "doing", "", "-5").unwrap();
        assert_eq!(o.reset_timer, Some(Duration::from_secs(10 * 60)));
        assert!(!o.write_journal);
    }

    #[test]
    fn manual_dismiss_with_invalid_interval_does_not_reset() {
        // Garbage interval on a manual Esc: still no error, still no reset
        // (the fallback interval is computed but unused because manual Esc
        // doesn't reschedule).
        let o = decide_close(TriggerSource::Manual, Action::Dismiss, "doing", "", "garbage")
            .unwrap();
        assert_eq!(o.reset_timer, None);
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
        // Spec §5: tray language is English. "Settings" sits between the
        // show and quit items so the menu reads visit-configure-bye.
        assert_eq!(TRAY_MENU_LABELS, ["Show Nudge", "Settings", "Quit"]);
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
