use crate::hotkey::Hotkey;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Recorder {
    #[default]
    Idle,
    Armed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PressEdge {
    pub key: PhysicalKey,
    pub modifiers: crate::hotkey::Modifiers,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PhysicalKey(pub u16);

pub trait EdgeSource {
    fn drain(&mut self) -> Vec<PressEdge>;
    fn set_armed(&mut self, armed: bool);
    fn classify(&self, edge: PressEdge) -> Verdict;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    Bindable(Hotkey),
    BareEscape,
    NotBindable { name: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Record,
    Cancel,
    FocusLost,
    Press(Verdict),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Nothing,
    Capture(Hotkey),
    Restore,
    Hint(String),
}

pub const PROMPT: &str = "Press modifiers plus one key. Esc cancels.";

pub fn step(state: Recorder, input: &Input) -> (Recorder, Effect) {
    match (state, input) {
        (Recorder::Idle, Input::Record) => (Recorder::Armed, Effect::Nothing),
        (Recorder::Idle, Input::Cancel) => (Recorder::Idle, Effect::Nothing),
        (Recorder::Idle, Input::FocusLost) => (Recorder::Idle, Effect::Nothing),
        (Recorder::Idle, Input::Press(_)) => (Recorder::Idle, Effect::Nothing),

        (Recorder::Armed, Input::Record) => (Recorder::Armed, Effect::Nothing),
        (Recorder::Armed, Input::Press(Verdict::Bindable(hk))) => {
            (Recorder::Idle, Effect::Capture(hk.clone()))
        }
        (Recorder::Armed, Input::Press(Verdict::BareEscape)) => (Recorder::Idle, Effect::Restore),
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

    fn apply(effect: &Effect, field: &str, saved: &str) -> String {
        match effect {
            Effect::Nothing | Effect::Hint(_) => field.to_string(),
            Effect::Capture(hk) => hotkey::format_native(hk),
            Effect::Restore => saved.to_string(),
        }
    }

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

    #[test]
    fn a_refused_press_leaves_the_recorder_able_to_record() {
        let (state, _) = step(Recorder::Armed, &refused("Keypad 5"));
        let (state, effect) = step(state, &bindable("Alt+Cmd+J"));
        assert_eq!(state, Recorder::Idle);
        assert_eq!(effect, Effect::Capture(hk("Alt+Cmd+J")));
    }

    #[test]
    fn every_cancelling_path_lands_on_the_saved_hotkey() {
        let saved = "Ctrl+Shift+Space";
        for input in [Input::Cancel, Input::FocusLost, bare_escape()] {
            let (state, effect) = step(Recorder::Armed, &input);
            assert_eq!(state, Recorder::Idle, "{input:?}");
            assert_eq!(apply(&effect, "Alt+J", saved), saved, "{input:?}");
        }
    }

    #[test]
    fn escape_with_a_modifier_records_instead_of_cancelling() {
        assert_eq!(
            step(Recorder::Armed, &bindable("Cmd+Escape")),
            (Recorder::Idle, Effect::Capture(hk("Cmd+Escape")))
        );
    }

    #[test]
    fn a_captured_combination_survives_a_round_trip_through_the_config() {
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
