//! The hotkey recorder's decision table: two states, six inputs, twelve cells,
//! no clock and no platform.
//!
//! Everything that decides lives in [`step`], a function of a state and one
//! input. **Whether a key is bindable is decided outside it and handed in
//! already answered** ([`Verdict`]), which is what keeps this module free of
//! every type that has ever made this feature rot: no key enum of a window
//! toolkit, no held-key set, no event loop, no `egui`.
//!
//! That freedom is structural rather than a habit. The previous attempt opened
//! with `use eframe::egui::{Key, Modifiers}` and the toolkit-to-token
//! translation table followed from it within the same file; here the signature
//! of [`step`] cannot accept a toolkit key at all.
//!
//! The transport types the platform and the UI share live here too
//! ([`PressEdge`], [`EdgeSource`]) — a press edge is the *only* keyboard input
//! an armed recorder accepts, so its shape belongs next to the table that
//! consumes it.

use crate::hotkey::Hotkey;

/// The recorder's whole state. Two values, no data — everything a cancelling
/// path needs is read back from the saved configuration, so there is nothing
/// to remember at the moment of arming.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Recorder {
    #[default]
    Idle,
    Armed,
}

/// One key going down, as the OS reports it.
///
/// `key` is the platform's own key code — on macOS the same integer the
/// registration call takes, which is why nothing sits between capture and
/// binding. `modifiers` is the [`crate::hotkey`] modifier mask (`MOD_CTRL` …
/// `MOD_WIN`) held at that instant, not a set of keys currently down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PressEdge {
    pub key: PhysicalKey,
    pub modifiers: u8,
}

/// A platform key code, opaque to everything above the platform layer. A
/// newtype rather than a bare `u16` so it cannot be confused with the modifier
/// mask it travels next to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhysicalKey(pub u16);

/// Where press edges come from. Narrow on purpose: the settings row drains it
/// once per frame and tells it whether to swallow, and that is the entire
/// contract a platform backend has to satisfy — which is what lets a recorded
/// event stream stand in for a keyboard in the test tier.
pub trait EdgeSource {
    /// Every press edge seen since the last call, oldest first.
    fn drain(&mut self) -> Vec<PressEdge>;
    /// While armed, the source swallows the keys it reports so they never
    /// reach the window: `⌘C` must not copy, `Esc` must not close, `Tab` must
    /// not move the focus.
    fn set_armed(&mut self, armed: bool);
    /// Judge one of its own edges against §5 — the "decided outside and handed
    /// in already answered" half of [`step`].
    ///
    /// It sits on the source because a key code only means anything next to
    /// the platform that produced it, and having the source answer keeps every
    /// OS-specific decision behind this one trait object: the settings row
    /// holds a `dyn EdgeSource` and has no `cfg` in it at all.
    fn classify(&self, edge: PressEdge) -> Verdict;
}

/// A press, already judged against §5 by the platform. The three answers are
/// exhaustive: a key either makes a hotkey, cancels, or is refused by name.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // only the macOS edge source hands out verdicts
pub enum Verdict {
    /// §5 accepts it and this platform can register it.
    Bindable(Hotkey),
    /// `Esc` with nothing held. A hotkey needs a modifier (except the F-row),
    /// so a lone `Esc` can never be one and the keystroke is free to mean
    /// "back out".
    BareEscape,
    /// Refused, with the name to show — `Keypad 5`, `J`, `fn`. The name is
    /// what makes the refusal legible, so it travels with the verdict.
    NotBindable { name: String },
}

/// What arrived at the row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// The `Record` button.
    Record,
    /// The `Cancel` button.
    Cancel,
    /// The window stopped being focused. Which shortcut took it is not
    /// observable — macOS consumes `⌘Tab` and `⌘Space` before any app sees
    /// them — so focus loss is the whole signal.
    FocusLost,
    /// A press edge, already classified.
    Press(Verdict),
}

/// What the row must do about it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Leave the row as it stands.
    Nothing,
    /// Put this combination in the field, spelled the way §5 writes it here.
    Capture(Hotkey),
    /// Put the saved hotkey back in the field.
    Restore,
    /// Stay armed, and show this line under the row.
    Hint(String),
}

/// The line an armed recorder shows until something happens.
pub const PROMPT: &str = "Press modifiers plus one key. Esc cancels.";

/// The §9 table, verbatim. Every cell is stated: no guards, no conditions
/// outside it, and no branch that reads anything but the state and the input.
///
/// | | Idle | Armed |
/// |---|---|---|
/// | **Record** | → Armed | — |
/// | **Press** — bindable | — | → Idle, field := the combination |
/// | **Press** — bare `Esc` | — | → Idle, field := the saved hotkey |
/// | **Press** — not bindable | — | → **Armed**, with a hint naming the key |
/// | **Cancel** | — | → Idle, field := the saved hotkey |
/// | **Focus lost** | — | → Idle, field := the saved hotkey |
pub fn step(state: Recorder, input: &Input) -> (Recorder, Effect) {
    match (state, input) {
        // --- Idle: only `Record` does anything. -----------------------------
        // A key pressed here belongs to whatever has the focus — in this row,
        // to the plain text field the user may be typing a hotkey into.
        (Recorder::Idle, Input::Record) => (Recorder::Armed, Effect::Nothing),
        (Recorder::Idle, Input::Cancel) => (Recorder::Idle, Effect::Nothing),
        (Recorder::Idle, Input::FocusLost) => (Recorder::Idle, Effect::Nothing),
        (Recorder::Idle, Input::Press(_)) => (Recorder::Idle, Effect::Nothing),

        // --- Armed ----------------------------------------------------------
        // `Record` is not on screen while armed; the button reads `Cancel`.
        (Recorder::Armed, Input::Record) => (Recorder::Armed, Effect::Nothing),
        (Recorder::Armed, Input::Press(Verdict::Bindable(hk))) => {
            (Recorder::Idle, Effect::Capture(hk.clone()))
        }
        (Recorder::Armed, Input::Press(Verdict::BareEscape)) => (Recorder::Idle, Effect::Restore),
        // Deliberately still Armed: disarming here would leave a mistyped
        // chord looking exactly like a recorded one.
        (Recorder::Armed, Input::Press(Verdict::NotBindable { name })) => (
            Recorder::Armed,
            Effect::Hint(format!("{name} cannot be part of a hotkey. {PROMPT}")),
        ),
        (Recorder::Armed, Input::Cancel) => (Recorder::Idle, Effect::Restore),
        (Recorder::Armed, Input::FocusLost) => (Recorder::Idle, Effect::Restore),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hotkey;

    fn hk(label: &str) -> Hotkey {
        hotkey::parse(label).expect("label must parse")
    }

    fn bindable(label: &str) -> Input {
        Input::Press(Verdict::Bindable(hk(label)))
    }

    fn refused(name: &str) -> Input {
        Input::Press(Verdict::NotBindable {
            name: name.to_string(),
        })
    }

    fn bare_escape() -> Input {
        Input::Press(Verdict::BareEscape)
    }

    /// What the field would read after `effect`, for a row whose saved hotkey
    /// is `saved` and which currently reads `field`.
    fn apply(effect: &Effect, field: &str, saved: &str) -> String {
        match effect {
            Effect::Nothing | Effect::Hint(_) => field.to_string(),
            Effect::Capture(hk) => hotkey::format_native(hk),
            Effect::Restore => saved.to_string(),
        }
    }

    // === the twelve cells ===================================================
    // One test per cell, in the order the table prints them. The Idle column
    // is "the recorder is not listening"; the Armed column is the machine.

    #[test]
    fn idle_record_arms() {
        assert_eq!(
            step(Recorder::Idle, &Input::Record),
            (Recorder::Armed, Effect::Nothing)
        );
    }

    #[test]
    fn idle_ignores_a_bindable_press() {
        assert_eq!(
            step(Recorder::Idle, &bindable("Alt+Cmd+J")),
            (Recorder::Idle, Effect::Nothing)
        );
    }

    #[test]
    fn idle_ignores_bare_escape() {
        // Esc in Idle belongs to the window — it closes it (§9 chrome).
        assert_eq!(
            step(Recorder::Idle, &bare_escape()),
            (Recorder::Idle, Effect::Nothing)
        );
    }

    #[test]
    fn idle_ignores_an_unbindable_press() {
        assert_eq!(
            step(Recorder::Idle, &refused("Keypad 5")),
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
        assert_eq!(
            step(Recorder::Armed, &Input::Record),
            (Recorder::Armed, Effect::Nothing)
        );
    }

    #[test]
    fn armed_captures_a_bindable_press() {
        assert_eq!(
            step(Recorder::Armed, &bindable("Alt+Cmd+J")),
            (Recorder::Idle, Effect::Capture(hk("Alt+Cmd+J")))
        );
    }

    #[test]
    fn armed_restores_on_bare_escape() {
        assert_eq!(
            step(Recorder::Armed, &bare_escape()),
            (Recorder::Idle, Effect::Restore)
        );
    }

    #[test]
    fn armed_stays_armed_on_an_unbindable_press() {
        let (state, effect) = step(Recorder::Armed, &refused("Keypad 5"));
        assert_eq!(
            state,
            Recorder::Armed,
            "disarming here would leave a mistyped chord looking recorded"
        );
        match effect {
            Effect::Hint(text) => assert!(
                text.contains("Keypad 5"),
                "the hint must name the key: {text}"
            ),
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

    // === what follows from the table, rather than being in it ===============

    #[test]
    fn a_refused_press_leaves_the_recorder_able_to_record() {
        // The property the "stays Armed" cell exists for: after a refusal the
        // very next good chord still lands, with no second Record click.
        let (state, _) = step(Recorder::Armed, &refused("Keypad 5"));
        let (state, effect) = step(state, &bindable("Alt+Cmd+J"));
        assert_eq!(state, Recorder::Idle);
        assert_eq!(effect, Effect::Capture(hk("Alt+Cmd+J")));
    }

    #[test]
    fn every_cancelling_path_lands_on_the_saved_hotkey() {
        // Not on a remembered earlier field value: cancelling has one meaning,
        // and a typed unsaved edit goes with it. This is what lets the
        // recorder hold no state at all when it arms.
        let saved = "Ctrl+Shift+Space";
        for input in [Input::Cancel, Input::FocusLost, bare_escape()] {
            let (state, effect) = step(Recorder::Armed, &input);
            assert_eq!(state, Recorder::Idle, "{input:?}");
            assert_eq!(apply(&effect, "Alt+J", saved), saved, "{input:?}");
        }
    }

    #[test]
    fn escape_with_a_modifier_records_instead_of_cancelling() {
        // `⌘Esc` is a hotkey like any other; only the bare press cancels, and
        // the two arrive as different verdicts rather than as one key the
        // table has to disambiguate.
        assert_eq!(
            step(Recorder::Armed, &bindable("Cmd+Escape")),
            (Recorder::Idle, Effect::Capture(hk("Cmd+Escape")))
        );
    }

    #[test]
    fn a_captured_combination_survives_a_round_trip_through_the_config() {
        // Save writes the field verbatim, so whatever the capture puts there
        // has to parse again at the next launch.
        let (_, effect) = step(Recorder::Armed, &bindable("Alt+Cmd+PageDown"));
        let Effect::Capture(captured) = effect else {
            panic!("expected a capture");
        };
        let label = hotkey::format_native(&captured);
        assert_eq!(
            hotkey::parse(&label).expect("a captured label must parse"),
            captured
        );
    }
}
