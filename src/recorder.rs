//! The hotkey recorder of design-spec §9: two states, six events, twelve
//! cells, no clock.
//!
//! Everything that decides lives in [`step`], which takes a state and an
//! input and returns the next state plus what the row must do. It touches no
//! `egui::Context` and reads no held-key set, so a test can hand it the exact
//! sequence a keyboard produces — which is the whole reason this is a
//! function and not a method on the settings window.

use eframe::egui::{Key, Modifiers};

use crate::hotkey;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Recorder {
    #[default]
    Idle,
    Armed,
}

/// What arrived at the row this frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Input {
    /// The `Record` button.
    Record,
    /// The `Cancel` button.
    Cancel,
    /// The window stopped being focused. Which shortcut took the focus is not
    /// knowable — see §9.
    FocusLost,
    /// A key going down, carrying the modifiers held at that instant. `key`
    /// is the physical cap wherever the toolkit reports one.
    Press {
        modifiers: Modifiers,
        key: Key,
        repeat: bool,
    },
    /// The toolkit turned a press into a clipboard command and the key never
    /// arrived. Which key it was is not recoverable — only that it cannot be
    /// recorded.
    ClipboardPress,
}

/// What the row must do about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Leave the row as it stands.
    Nothing,
    /// Put this label in the field. Already spelled the way §5 writes it for
    /// this platform.
    Capture(String),
    /// Put the saved hotkey back in the field.
    Restore,
    /// Stay armed, and show this line in place of the prompt.
    Hint(String),
}

/// The line an armed recorder shows until something happens.
pub const PROMPT: &str = "Modifiers plus one key. Esc cancels.";

/// The six events of the §9 table. A press is classified into one of three
/// before the table is consulted, because "a press edge" is defined by what
/// it is not — not a repeat, not a release, not a held set — and that
/// definition belongs next to the table rather than inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum TableEvent {
    Record,
    Cancel,
    FocusLost,
    /// §5 can bind it: the canonical label.
    PressBindable(String),
    /// Bare `Esc` — a hotkey is modifiers *plus* a key, so a lone `Esc` can
    /// never be one and the keystroke is free to mean "back out".
    PressCancels,
    /// §5 cannot bind it: the hint naming what was pressed.
    PressRefused(String),
    /// Not a press edge at all, and so not an event.
    NotAnEvent,
}

fn classify(input: &Input) -> TableEvent {
    match *input {
        Input::Record => TableEvent::Record,
        Input::Cancel => TableEvent::Cancel,
        Input::FocusLost => TableEvent::FocusLost,
        Input::ClipboardPress => TableEvent::PressRefused(
            "Cut, copy and paste cannot be recorded — type that one in the field. Esc cancels."
                .to_string(),
        ),
        // A held key repeating is the same press, still down.
        Input::Press { repeat: true, .. } => TableEvent::NotAnEvent,
        Input::Press { modifiers, key, .. } => {
            if key == Key::Escape && !any_modifier(modifiers) {
                return TableEvent::PressCancels;
            }
            match hotkey::hotkey_from_egui(modifiers, key) {
                Some(hk) => TableEvent::PressBindable(hotkey::format_native(&hk)),
                None => TableEvent::PressRefused(format!(
                    "{} cannot be part of a hotkey. {PROMPT}",
                    key.name()
                )),
            }
        }
    }
}

/// True if any of the four modifiers is down. `command` is folded in because
/// on non-mac targets it *is* ctrl, and on mac `mac_cmd` carries the same
/// press — a bare `Esc` must mean bare on both.
fn any_modifier(m: Modifiers) -> bool {
    m.ctrl || m.alt || m.shift || m.mac_cmd || m.command
}

/// The §9 table. Every cell is stated; there are no guards and no conditions
/// outside it.
pub fn step(state: Recorder, input: &Input) -> (Recorder, Effect) {
    match (state, classify(input)) {
        // Idle: only `Record` does anything. A key pressed here belongs to
        // whatever else has the focus.
        (Recorder::Idle, TableEvent::Record) => (Recorder::Armed, Effect::Nothing),
        (Recorder::Idle, _) => (Recorder::Idle, Effect::Nothing),

        // Armed.
        (Recorder::Armed, TableEvent::PressBindable(label)) => {
            (Recorder::Idle, Effect::Capture(label))
        }
        (Recorder::Armed, TableEvent::PressCancels) => (Recorder::Idle, Effect::Restore),
        (Recorder::Armed, TableEvent::PressRefused(hint)) => (Recorder::Armed, Effect::Hint(hint)),
        (Recorder::Armed, TableEvent::Cancel) => (Recorder::Idle, Effect::Restore),
        (Recorder::Armed, TableEvent::FocusLost) => (Recorder::Idle, Effect::Restore),
        // `Record` is not on screen while armed, and a repeat is not a press.
        (Recorder::Armed, TableEvent::Record | TableEvent::NotAnEvent) => {
            (Recorder::Armed, Effect::Nothing)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn press(modifiers: Modifiers, key: Key) -> Input {
        Input::Press {
            modifiers,
            key,
            repeat: false,
        }
    }

    fn alt_cmd() -> Modifiers {
        Modifiers {
            alt: true,
            mac_cmd: true,
            command: true,
            ..Modifiers::NONE
        }
    }

    /// What the field would read after `effect`, for a row whose saved hotkey
    /// is `saved` and which currently reads `field`.
    fn apply(effect: &Effect, field: &str, saved: &str) -> String {
        match effect {
            Effect::Nothing | Effect::Hint(_) => field.to_string(),
            Effect::Capture(label) => label.clone(),
            Effect::Restore => saved.to_string(),
        }
    }

    // === the twelve cells ====================================================
    // Row order follows the §9 table. The Idle column is six cells of "the
    // recorder is not listening"; the Armed column is where the machine is.

    #[test]
    fn idle_record_arms() {
        assert_eq!(
            step(Recorder::Idle, &Input::Record),
            (Recorder::Armed, Effect::Nothing)
        );
    }

    #[test]
    fn idle_ignores_a_bindable_press() {
        // The field is a plain text box in Idle and owns its own keystrokes;
        // the recorder must not reach across and capture them.
        assert_eq!(
            step(Recorder::Idle, &press(alt_cmd(), Key::J)),
            (Recorder::Idle, Effect::Nothing)
        );
    }

    #[test]
    fn idle_ignores_bare_escape() {
        // Esc in Idle belongs to the window: it is Cancel there (§9 chrome).
        assert_eq!(
            step(Recorder::Idle, &press(Modifiers::NONE, Key::Escape)),
            (Recorder::Idle, Effect::Nothing)
        );
    }

    #[test]
    fn idle_ignores_an_unbindable_press() {
        assert_eq!(
            step(Recorder::Idle, &press(Modifiers::CTRL, Key::F25)),
            (Recorder::Idle, Effect::Nothing)
        );
    }

    #[test]
    fn idle_ignores_cancel() {
        assert_eq!(
            step(Recorder::Idle, &Input::Cancel),
            (Recorder::Idle, Effect::Nothing)
        );
    }

    #[test]
    fn idle_ignores_focus_loss() {
        assert_eq!(
            step(Recorder::Idle, &Input::FocusLost),
            (Recorder::Idle, Effect::Nothing)
        );
    }

    #[test]
    fn armed_ignores_record() {
        // `Record` is not on screen while armed — the button reads `Cancel`.
        assert_eq!(
            step(Recorder::Armed, &Input::Record),
            (Recorder::Armed, Effect::Nothing)
        );
    }

    #[test]
    fn armed_captures_a_bindable_press() {
        let (state, effect) = step(Recorder::Armed, &press(alt_cmd(), Key::J));
        assert_eq!(state, Recorder::Idle);
        let label = hotkey::format_native(&hotkey::parse("Alt+Cmd+J").unwrap());
        assert_eq!(effect, Effect::Capture(label));
    }

    #[test]
    fn armed_restores_on_bare_escape() {
        assert_eq!(
            step(Recorder::Armed, &press(Modifiers::NONE, Key::Escape)),
            (Recorder::Idle, Effect::Restore)
        );
    }

    #[test]
    fn armed_stays_armed_on_an_unbindable_press() {
        let (state, effect) = step(Recorder::Armed, &press(Modifiers::CTRL, Key::F25));
        assert_eq!(state, Recorder::Armed, "disarming here would leave a mistyped chord looking recorded");
        match effect {
            Effect::Hint(text) => assert!(text.contains("F25"), "the hint must name the key: {text}"),
            other => panic!("expected a hint, got {other:?}"),
        }
    }

    #[test]
    fn armed_restores_on_cancel() {
        assert_eq!(
            step(Recorder::Armed, &Input::Cancel),
            (Recorder::Idle, Effect::Restore)
        );
    }

    #[test]
    fn armed_restores_on_focus_loss() {
        assert_eq!(
            step(Recorder::Armed, &Input::FocusLost),
            (Recorder::Idle, Effect::Restore)
        );
    }

    // === what a press edge is not ============================================

    #[test]
    fn a_repeat_is_not_a_press() {
        // A key held a beat too long repeats. It is the same press, still
        // down, and re-capturing on it would fight the user's finger.
        let repeat = Input::Press {
            modifiers: alt_cmd(),
            key: Key::J,
            repeat: true,
        };
        assert_eq!(
            step(Recorder::Armed, &repeat),
            (Recorder::Armed, Effect::Nothing)
        );
    }

    #[test]
    fn a_clipboard_command_is_a_refusal_not_a_silence() {
        // ⌘C never arrives as a key — the toolkit turns it into a copy. Doing
        // nothing would read as a dead recorder, which is the failure this
        // feature keeps returning to.
        let (state, effect) = step(Recorder::Armed, &Input::ClipboardPress);
        assert_eq!(state, Recorder::Armed);
        assert!(matches!(effect, Effect::Hint(_)), "got {effect:?}");
    }

    // === the §9 acceptance list, as far as a function can carry it ==========
    // The rest of it is a real keyboard: what the toolkit delivers is exactly
    // what these tests assume, and assuming it is how this feature failed
    // twice.

    #[test]
    fn the_acceptance_combinations_land_spelled_the_way_5_writes_them() {
        let shift = Modifiers {
            shift: true,
            ..Modifiers::NONE
        };
        let cases = [
            (alt_cmd(), Key::J, "Alt+Cmd+J"),
            (alt_cmd(), Key::ArrowLeft, "Alt+Cmd+Left"),
            (shift, Key::Slash, "Shift+/"),
        ];
        for (mods, key, expected) in cases {
            let (_, effect) = step(Recorder::Armed, &press(mods, key));
            let want = hotkey::format_native(&hotkey::parse(expected).unwrap());
            assert_eq!(effect, Effect::Capture(want), "{expected}");
        }
    }

    #[test]
    fn a_shifted_cap_records_as_the_cap() {
        // `⇧/` and `?` are one key. Whichever the toolkit names, the label is
        // the unshifted legend.
        let shift = Modifiers {
            shift: true,
            ..Modifiers::NONE
        };
        let (_, from_slash) = step(Recorder::Armed, &press(shift, Key::Slash));
        let (_, from_question) = step(Recorder::Armed, &press(shift, Key::Questionmark));
        assert_eq!(from_slash, from_question);
    }

    #[test]
    fn escape_with_a_modifier_records_instead_of_cancelling() {
        let (state, effect) = step(Recorder::Armed, &press(Modifiers::CTRL, Key::Escape));
        assert_eq!(state, Recorder::Idle);
        let want = hotkey::format_native(&hotkey::parse("Ctrl+Esc").unwrap());
        assert_eq!(effect, Effect::Capture(want));
    }

    #[test]
    fn every_cancelling_path_lands_on_the_saved_hotkey() {
        // Not on a remembered earlier field value: cancelling has one
        // meaning, and a typed unsaved edit goes with it.
        let saved = "Ctrl+Shift+Space";
        for input in [
            Input::Cancel,
            Input::FocusLost,
            press(Modifiers::NONE, Key::Escape),
        ] {
            let (_, effect) = step(Recorder::Armed, &input);
            assert_eq!(apply(&effect, "Alt+J", saved), saved, "{input:?}");
        }
    }

    #[test]
    fn a_captured_label_survives_a_round_trip_through_the_config() {
        // Save writes the field verbatim, so whatever lands there has to
        // parse again on the next launch.
        let (_, effect) = step(Recorder::Armed, &press(alt_cmd(), Key::PageDown));
        let Effect::Capture(label) = effect else {
            panic!("expected a capture");
        };
        let parsed = hotkey::parse(&label).expect("a captured label must parse");
        assert_eq!(hotkey::format_native(&parsed), label);
    }
}
