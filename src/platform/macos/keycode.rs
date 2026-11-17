use crate::hotkey::{self, Modifiers};
use crate::recorder::{PhysicalKey, PressEdge, Verdict};

#[rustfmt::skip]
pub const KEYS: &[(u16, &str)] = &[
    (0x00, "A"), (0x01, "S"), (0x02, "D"), (0x03, "F"), (0x04, "H"), (0x05, "G"),
    (0x06, "Z"), (0x07, "X"), (0x08, "C"), (0x09, "V"), (0x0B, "B"), (0x0C, "Q"),
    (0x0D, "W"), (0x0E, "E"), (0x0F, "R"), (0x10, "Y"), (0x11, "T"), (0x1F, "O"),
    (0x20, "U"), (0x22, "I"), (0x23, "P"), (0x25, "L"), (0x26, "J"), (0x28, "K"),
    (0x2D, "N"), (0x2E, "M"),
    (0x12, "1"), (0x13, "2"), (0x14, "3"), (0x15, "4"), (0x16, "6"), (0x17, "5"),
    (0x19, "9"), (0x1A, "7"), (0x1C, "8"), (0x1D, "0"),
    (0x18, "="), (0x1B, "-"), (0x1E, "]"), (0x21, "["), (0x27, "'"), (0x29, ";"),
    (0x2A, "\\"), (0x2B, ","), (0x2C, "/"), (0x2F, "."), (0x32, "`"),
    (0x24, "ENTER"), (0x30, "TAB"), (0x31, "SPACE"), (0x33, "BACKSPACE"),
    (0x35, "ESCAPE"), (0x75, "DELETE"),
    (0x72, "INSERT"), (0x73, "HOME"), (0x74, "PAGEUP"), (0x77, "END"), (0x79, "PAGEDOWN"),
    (0x7B, "LEFT"), (0x7C, "RIGHT"), (0x7D, "DOWN"), (0x7E, "UP"),
    (0x7A, "F1"), (0x78, "F2"), (0x63, "F3"), (0x76, "F4"), (0x60, "F5"),
    (0x61, "F6"), (0x62, "F7"), (0x64, "F8"), (0x65, "F9"), (0x6D, "F10"),
    (0x67, "F11"), (0x6F, "F12"), (0x69, "F13"), (0x6B, "F14"), (0x71, "F15"),
    (0x6A, "F16"), (0x40, "F17"), (0x4F, "F18"), (0x50, "F19"), (0x5A, "F20"),
];

pub const TOKENS_WITHOUT_A_KEY_CODE: &[&str] = &["F21", "F22", "F23", "F24"];

#[rustfmt::skip]
pub const REFUSED: &[(u16, &str)] = &[
    (0x52, "Keypad 0"), (0x53, "Keypad 1"), (0x54, "Keypad 2"), (0x55, "Keypad 3"),
    (0x56, "Keypad 4"), (0x57, "Keypad 5"), (0x58, "Keypad 6"), (0x59, "Keypad 7"),
    (0x5B, "Keypad 8"), (0x5C, "Keypad 9"),
    (0x41, "Keypad ."), (0x43, "Keypad *"), (0x45, "Keypad +"), (0x47, "Keypad Clear"),
    (0x4B, "Keypad /"), (0x4C, "Keypad Enter"), (0x4E, "Keypad -"), (0x51, "Keypad ="),
    (0x37, "Command"), (0x38, "Shift"), (0x39, "Caps Lock"), (0x3A, "Option"),
    (0x3B, "Control"), (0x3C, "Right Shift"), (0x3D, "Right Option"),
    (0x3E, "Right Control"),
    (0x3F, "fn"),
    (0x48, "Volume Up"), (0x49, "Volume Down"), (0x4A, "Mute"),
    (0x0A, "§"), (0x5D, "¥"), (0x5E, "_"), (0x5F, "Keypad ,"),
    (0x66, "英数"), (0x68, "かな"),
];

pub fn token_for(code: u16) -> Option<&'static str> {
    KEYS.iter()
        .find(|(c, _)| *c == code)
        .map(|(_, token)| *token)
}

pub fn keycode_for(token: &str) -> Option<u16> {
    KEYS.iter()
        .find(|(_, t)| *t == token)
        .map(|(code, _)| *code)
}

fn refused_name(code: u16) -> String {
    REFUSED
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, name)| (*name).to_string())
        .unwrap_or_else(|| format!("Key code {code}"))
}

pub fn modifiers_from_ns_flags(flags: u64) -> Modifiers {
    const NS_SHIFT: u64 = 1 << 17;
    const NS_CONTROL: u64 = 1 << 18;
    const NS_OPTION: u64 = 1 << 19;
    const NS_COMMAND: u64 = 1 << 20;

    let mut out = Modifiers::NONE;
    if flags & NS_CONTROL != 0 {
        out |= Modifiers::CTRL;
    }
    if flags & NS_OPTION != 0 {
        out |= Modifiers::ALT;
    }
    if flags & NS_SHIFT != 0 {
        out |= Modifiers::SHIFT;
    }
    if flags & NS_COMMAND != 0 {
        out |= Modifiers::WIN;
    }
    out
}

pub fn carbon_modifiers(m: Modifiers) -> u32 {
    const CMD_KEY: u32 = 1 << 8;
    const SHIFT_KEY: u32 = 1 << 9;
    const OPTION_KEY: u32 = 1 << 11;
    const CONTROL_KEY: u32 = 1 << 12;

    let mut out = 0u32;
    if m.contains(Modifiers::CTRL) {
        out |= CONTROL_KEY;
    }
    if m.contains(Modifiers::ALT) {
        out |= OPTION_KEY;
    }
    if m.contains(Modifiers::SHIFT) {
        out |= SHIFT_KEY;
    }
    if m.contains(Modifiers::WIN) {
        out |= CMD_KEY;
    }
    out
}

pub fn classify_press(edge: PressEdge) -> Verdict {
    let PhysicalKey(code) = edge.key;
    let Some(token) = token_for(code) else {
        return Verdict::NotBindable {
            name: refused_name(code),
        };
    };
    if token == "ESCAPE" && edge.modifiers.is_empty() {
        return Verdict::BareEscape;
    }
    match hotkey::from_parts(edge.modifiers, token) {
        Ok(hk) => Verdict::Bindable(hk),
        Err(_) => Verdict::NotBindable {
            name: hotkey::key_label(token),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn edge(code: u16, modifiers: Modifiers) -> PressEdge {
        PressEdge {
            key: PhysicalKey(code),
            modifiers,
        }
    }

    #[test]
    fn every_token_the_grammar_accepts_has_exactly_one_key_code() {
        let mut missing: Vec<String> = Vec::new();
        for token in hotkey::all_key_tokens() {
            match keycode_for(&token) {
                Some(code) => assert_eq!(
                    token_for(code),
                    Some(token.as_str()),
                    "{token} maps to {code}, which maps back to something else"
                ),
                None => missing.push(token),
            }
        }
        assert_eq!(
            missing, TOKENS_WITHOUT_A_KEY_CODE,
            "the tokens without a macOS key code must be exactly the documented ones — \
             anything else is a hotkey a user can save and the OS will never fire"
        );
    }

    #[test]
    fn every_key_code_in_the_table_maps_to_a_token_the_parser_accepts() {
        for (code, token) in KEYS {
            let hk = hotkey::parse(&format!("Ctrl+{token}")).unwrap_or_else(|e| {
                panic!("key code {code} names {token}, which does not parse: {e}")
            });
            assert_eq!(
                hk.key.as_str(),
                *token,
                "key code {code} names {token}, which is not the canonical spelling"
            );
        }
    }

    #[test]
    fn no_key_code_and_no_token_appears_twice() {
        let mut codes = HashSet::new();
        let mut tokens = HashSet::new();
        for (code, token) in KEYS {
            assert!(codes.insert(*code), "key code {code} listed twice");
            assert!(tokens.insert(*token), "token {token} listed twice");
        }
    }

    #[test]
    fn the_bindable_and_the_refused_share_no_key_code() {
        let bindable: HashSet<u16> = KEYS.iter().map(|(c, _)| *c).collect();
        for (code, name) in REFUSED {
            assert!(
                !bindable.contains(code),
                "{name} ({code}) is refused and bindable at once"
            );
        }
        let mut seen = HashSet::new();
        for (code, name) in REFUSED {
            assert!(seen.insert(*code), "{name} ({code}) listed twice");
        }
    }

    #[test]
    fn ns_flags_read_the_four_modifiers_carbon_can_bind() {
        assert_eq!(modifiers_from_ns_flags(0), Modifiers::NONE);
        assert_eq!(modifiers_from_ns_flags(1 << 18), Modifiers::CTRL);
        assert_eq!(modifiers_from_ns_flags(1 << 19), Modifiers::ALT);
        assert_eq!(modifiers_from_ns_flags(1 << 17), Modifiers::SHIFT);
        assert_eq!(modifiers_from_ns_flags(1 << 20), Modifiers::WIN);
        assert_eq!(
            modifiers_from_ns_flags((1 << 19) | (1 << 20)),
            Modifiers::ALT | Modifiers::WIN
        );
    }

    #[test]
    fn the_flags_macos_sets_by_itself_are_not_modifiers() {
        for stray in [1 << 16, 1 << 21, 1 << 22, 1 << 23] {
            assert_eq!(
                modifiers_from_ns_flags(stray),
                Modifiers::NONE,
                "flag {stray:#x}"
            );
            assert_eq!(
                modifiers_from_ns_flags((1 << 20) | stray),
                Modifiers::WIN,
                "flag {stray:#x} next to Command"
            );
        }
    }

    #[test]
    fn carbon_modifiers_are_the_documented_bits() {
        assert_eq!(carbon_modifiers(Modifiers::NONE), 0);
        assert_eq!(carbon_modifiers(Modifiers::CTRL), 1 << 12);
        assert_eq!(carbon_modifiers(Modifiers::ALT), 1 << 11);
        assert_eq!(carbon_modifiers(Modifiers::SHIFT), 1 << 9);
        assert_eq!(carbon_modifiers(Modifiers::WIN), 1 << 8);
        assert_eq!(
            carbon_modifiers(Modifiers::CTRL | Modifiers::ALT | Modifiers::SHIFT | Modifiers::WIN),
            (1 << 12) | (1 << 11) | (1 << 9) | (1 << 8)
        );
    }

    #[test]
    fn a_modified_letter_is_bindable() {
        let verdict = classify_press(edge(0x26, Modifiers::ALT | Modifiers::WIN));
        assert_eq!(
            verdict,
            Verdict::Bindable(hotkey::parse("Alt+Cmd+J").unwrap())
        );
    }

    #[test]
    fn a_bare_letter_is_refused_by_name() {
        match classify_press(edge(0x26, Modifiers::NONE)) {
            Verdict::NotBindable { name } => assert_eq!(name, "J"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_bare_f_key_records() {
        assert_eq!(
            classify_press(edge(0x69, Modifiers::NONE)),
            Verdict::Bindable(hotkey::parse("F13").unwrap())
        );
    }

    #[test]
    fn bare_escape_cancels_and_a_modified_escape_records() {
        assert_eq!(
            classify_press(edge(0x35, Modifiers::NONE)),
            Verdict::BareEscape
        );
        assert_eq!(
            classify_press(edge(0x35, Modifiers::WIN)),
            Verdict::Bindable(hotkey::parse("Cmd+Escape").unwrap())
        );
    }

    #[test]
    fn the_keypad_is_refused_by_name_and_is_not_the_digit_row() {
        assert_ne!(keycode_for("5"), Some(0x57));
        match classify_press(edge(0x57, Modifiers::WIN)) {
            Verdict::NotBindable { name } => assert_eq!(name, "Keypad 5"),
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert_eq!(
            classify_press(edge(keycode_for("5").unwrap(), Modifiers::WIN)),
            Verdict::Bindable(hotkey::parse("Cmd+5").unwrap())
        );
    }

    #[test]
    fn an_unknown_key_code_still_says_that_the_press_arrived() {
        match classify_press(edge(0xFF, Modifiers::WIN)) {
            Verdict::NotBindable { name } => assert!(name.contains("255"), "{name}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn the_clipboard_chords_are_ordinary_bindable_keys_here() {
        for (code, label) in [(0x07, "Cmd+X"), (0x08, "Cmd+C"), (0x09, "Cmd+V")] {
            assert_eq!(
                classify_press(edge(code, Modifiers::WIN)),
                Verdict::Bindable(hotkey::parse(label).unwrap()),
                "{label}"
            );
        }
    }
}
