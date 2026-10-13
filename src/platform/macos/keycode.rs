//! The one table: a macOS virtual key code ↔ a §5 key token, used in both
//! directions.
//!
//! This is the module the whole task exists for. The key code an `NSEvent`
//! carries is the key code `RegisterEventHotKey` takes, so reading a key and
//! binding it are the same integer and there is nothing between them to
//! disagree. Every earlier attempt had two tables — one from the window
//! toolkit's key type to ours, one from ours to the registration call — and
//! every bug lived in the gap.
//!
//! [`KEYS`] is that single array; [`token_for`] and [`keycode_for`] are
//! derived from it, so the two directions cannot drift.
//!
//! [`REFUSED`] is a different kind of thing and is kept strictly separate: it
//! maps a key code the OS reports to a **human name**, so a refusal can say
//! "Keypad 5" instead of "87". It has no counterpart on the registration side
//! to disagree with, nothing downstream of it can produce a `Hotkey`, and a
//! test asserts the two arrays share no key code. It is not the translation
//! table this task removed.
//!
//! The numbers are `kVK_*` from HIToolbox's `Events.h`. They are positions on
//! the keyboard, not characters: `kVK_ANSI_J` is the cap where `J` sits on a
//! US layout, and it is still that cap when a Russian layout makes it type
//! `о`. That is what §5 rule 7 promises and it costs nothing to keep here.

use crate::hotkey::{self, MOD_ALT, MOD_CTRL, MOD_SHIFT, MOD_WIN};
use crate::recorder::{PhysicalKey, PressEdge, Verdict};

/// Every key §5 can name, as `(kVK_* code, canonical token)`.
///
/// Total against [`hotkey::all_key_tokens`] except for `F21`–`F24`, which the
/// grammar accepts and macOS has no `kVK_*` constant for — see
/// [`TOKENS_WITHOUT_A_KEY_CODE`].
#[rustfmt::skip]
pub const KEYS: &[(u16, &str)] = &[
    // Letters, in key-code order rather than alphabetical — this is the
    // physical layout of a US board, and reading it in code order is how a
    // typo gets spotted against `Events.h`.
    (0x00, "A"), (0x01, "S"), (0x02, "D"), (0x03, "F"), (0x04, "H"), (0x05, "G"),
    (0x06, "Z"), (0x07, "X"), (0x08, "C"), (0x09, "V"), (0x0B, "B"), (0x0C, "Q"),
    (0x0D, "W"), (0x0E, "E"), (0x0F, "R"), (0x10, "Y"), (0x11, "T"), (0x1F, "O"),
    (0x20, "U"), (0x22, "I"), (0x23, "P"), (0x25, "L"), (0x26, "J"), (0x28, "K"),
    (0x2D, "N"), (0x2E, "M"),
    // Digit row.
    (0x12, "1"), (0x13, "2"), (0x14, "3"), (0x15, "4"), (0x16, "6"), (0x17, "5"),
    (0x19, "9"), (0x1A, "7"), (0x1C, "8"), (0x1D, "0"),
    // Symbol caps, named by their unshifted legend (§5 rule 6).
    (0x18, "="), (0x1B, "-"), (0x1E, "]"), (0x21, "["), (0x27, "'"), (0x29, ";"),
    (0x2A, "\\"), (0x2B, ","), (0x2C, "/"), (0x2F, "."), (0x32, "`"),
    // Named keys. `DELETE` is forward delete (⌦), a different key from
    // `BACKSPACE` (⌫) whatever the Mac keycap says.
    (0x24, "ENTER"), (0x30, "TAB"), (0x31, "SPACE"), (0x33, "BACKSPACE"),
    (0x35, "ESCAPE"), (0x75, "DELETE"),
    // Navigation. `INSERT` is `kVK_Help`: an Insert key on a PC keyboard
    // plugged into a Mac reports this code, and an Apple keyboard has no cap
    // for it at all.
    (0x72, "INSERT"), (0x73, "HOME"), (0x74, "PAGEUP"), (0x77, "END"), (0x79, "PAGEDOWN"),
    // Arrows.
    (0x7B, "LEFT"), (0x7C, "RIGHT"), (0x7D, "DOWN"), (0x7E, "UP"),
    // The F row, in the scattered order Apple assigned it.
    (0x7A, "F1"), (0x78, "F2"), (0x63, "F3"), (0x76, "F4"), (0x60, "F5"),
    (0x61, "F6"), (0x62, "F7"), (0x64, "F8"), (0x65, "F9"), (0x6D, "F10"),
    (0x67, "F11"), (0x6F, "F12"), (0x69, "F13"), (0x6B, "F14"), (0x71, "F15"),
    (0x6A, "F16"), (0x40, "F17"), (0x4F, "F18"), (0x50, "F19"), (0x5A, "F20"),
];

/// The tokens §5 accepts that this platform has no key code for.
///
/// `F21`–`F24` exist in the grammar because Windows can bind them. macOS
/// defines no `kVK_*` past `F20` and whether it would bind one has never been
/// measured, so they are refused here rather than guessed at — a guessed code
/// registers happily (registration validates nothing) and then never fires.
#[allow(dead_code)] // named by the totality test; the app only reads `KEYS`
pub const TOKENS_WITHOUT_A_KEY_CODE: &[&str] = &["F21", "F22", "F23", "F24"];

/// Key codes that are not in the grammar but must be **refused by name**.
///
/// A recorder that answers a keypad press with silence reads as broken, and
/// one that answers it with "87" reads as contempt. Nothing here can become a
/// `Hotkey`: this array feeds hint text only.
#[rustfmt::skip]
pub const REFUSED: &[(u16, &str)] = &[
    // The keypad. It registers perfectly well; §5 leaves it out to keep the
    // vocabulary one name per key, and the digits are the reason — keypad 5
    // and the digit row's 5 are different keys with different codes, and one
    // spelling cannot mean both.
    (0x52, "Keypad 0"), (0x53, "Keypad 1"), (0x54, "Keypad 2"), (0x55, "Keypad 3"),
    (0x56, "Keypad 4"), (0x57, "Keypad 5"), (0x58, "Keypad 6"), (0x59, "Keypad 7"),
    (0x5B, "Keypad 8"), (0x5C, "Keypad 9"),
    (0x41, "Keypad ."), (0x43, "Keypad *"), (0x45, "Keypad +"), (0x47, "Keypad Clear"),
    (0x4B, "Keypad /"), (0x4C, "Keypad Enter"), (0x4E, "Keypad -"), (0x51, "Keypad ="),
    // The modifiers themselves. These arrive as `flagsChanged`, which the
    // monitor's mask excludes, so they are unreachable through the recorder —
    // listed so that a future widening of the mask names them instead of
    // printing a number.
    (0x37, "Command"), (0x38, "Shift"), (0x39, "Caps Lock"), (0x3A, "Option"),
    (0x3B, "Control"), (0x3C, "Right Shift"), (0x3D, "Right Option"),
    (0x3E, "Right Control"),
    // `fn`. Not a modifier a hotkey may carry (§5): Carbon takes four modifier
    // bits and this is not one of them.
    (0x3F, "fn"),
    // Media keys. Reachable only through an event tap, which needs a
    // permission this app does not ask for.
    (0x48, "Volume Up"), (0x49, "Volume Down"), (0x4A, "Mute"),
    // Layout-specific caps with no §5 name.
    (0x0A, "§"), (0x5D, "¥"), (0x5E, "_"), (0x5F, "Keypad ,"),
    (0x66, "英数"), (0x68, "かな"),
];

/// The §5 token for a macOS key code, or `None` if this key is outside the
/// grammar.
pub fn token_for(code: u16) -> Option<&'static str> {
    KEYS.iter()
        .find(|(c, _)| *c == code)
        .map(|(_, token)| *token)
}

/// The macOS key code for a canonical §5 token, or `None` for a token this
/// platform cannot bind (see [`TOKENS_WITHOUT_A_KEY_CODE`]).
pub fn keycode_for(token: &str) -> Option<u16> {
    KEYS.iter()
        .find(|(_, t)| *t == token)
        .map(|(code, _)| *code)
}

/// The name to show for a key that cannot be recorded.
fn refused_name(code: u16) -> String {
    REFUSED
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, name)| (*name).to_string())
        // Not every key on every keyboard is worth a row above, and a number
        // is still a name: it tells the user their press *arrived*, which is
        // the thing a silent recorder fails to say.
        .unwrap_or_else(|| format!("Key code {code}"))
}

/// Our modifier mask from an `NSEvent`'s `modifierFlags`.
///
/// Only the four Carbon can bind are read. `capsLock`, `numericPad`, `help`
/// and `function` are deliberately dropped: macOS sets `function` by itself
/// for the arrows and the F row, so honouring it would make `⌥⌘←` a different
/// combination from the one the user pressed.
pub fn modifiers_from_ns_flags(flags: u64) -> u8 {
    const NS_SHIFT: u64 = 1 << 17;
    const NS_CONTROL: u64 = 1 << 18;
    const NS_OPTION: u64 = 1 << 19;
    const NS_COMMAND: u64 = 1 << 20;

    let mut out = 0u8;
    if flags & NS_CONTROL != 0 {
        out |= MOD_CTRL;
    }
    if flags & NS_OPTION != 0 {
        out |= MOD_ALT;
    }
    if flags & NS_SHIFT != 0 {
        out |= MOD_SHIFT;
    }
    if flags & NS_COMMAND != 0 {
        out |= MOD_WIN;
    }
    out
}

/// Our modifier mask as Carbon's, the shape `RegisterEventHotKey` takes.
/// `MOD_WIN` is Command here, matching the parser's `Win|Super|Meta|Cmd`
/// aliasing onto one bit.
pub fn carbon_modifiers(m: u8) -> u32 {
    const CMD_KEY: u32 = 1 << 8;
    const SHIFT_KEY: u32 = 1 << 9;
    const OPTION_KEY: u32 = 1 << 11;
    const CONTROL_KEY: u32 = 1 << 12;

    let mut out = 0u32;
    if m & MOD_CTRL != 0 {
        out |= CONTROL_KEY;
    }
    if m & MOD_ALT != 0 {
        out |= OPTION_KEY;
    }
    if m & MOD_SHIFT != 0 {
        out |= SHIFT_KEY;
    }
    if m & MOD_WIN != 0 {
        out |= CMD_KEY;
    }
    out
}

/// Judge one press edge against §5 — the "decided outside and handed in"
/// half of the recorder's table.
///
/// "Bindable" means exactly one thing here: the grammar accepts it *and*
/// [`keycode_for`] can hand the same code back to the registration call. Both
/// halves are checked in this one function, so there is no combination the
/// recorder can capture and the binder then refuse.
pub fn classify_press(edge: PressEdge) -> Verdict {
    let PhysicalKey(code) = edge.key;
    let Some(token) = token_for(code) else {
        return Verdict::NotBindable {
            name: refused_name(code),
        };
    };
    // A hotkey is modifiers *plus* a key, so a lone Esc can never be one and
    // the keystroke is free to mean "back out". With anything held it is an
    // ordinary combination and records.
    if token == "ESCAPE" && edge.modifiers == 0 {
        return Verdict::BareEscape;
    }
    match hotkey::from_parts(edge.modifiers, token) {
        Ok(hk) => Verdict::Bindable(hk),
        // The only error `from_parts` can return for a token that came out of
        // `KEYS` is the missing-modifier rule; name the key, since that is
        // what the user has to fix.
        Err(_) => Verdict::NotBindable {
            name: hotkey::key_label(token),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn edge(code: u16, modifiers: u8) -> PressEdge {
        PressEdge {
            key: PhysicalKey(code),
            modifiers,
        }
    }

    // === totality and agreement ==============================================
    //
    // These are the tests that could not exist while there were two tables,
    // and they are the reason this module is shaped as one array.

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
        // The one way `REFUSED` could turn into the banned second table is by
        // claiming a key the real one already owns.
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

    // === the modifier nibble =================================================

    #[test]
    fn ns_flags_read_the_four_modifiers_carbon_can_bind() {
        assert_eq!(modifiers_from_ns_flags(0), 0);
        assert_eq!(modifiers_from_ns_flags(1 << 18), MOD_CTRL);
        assert_eq!(modifiers_from_ns_flags(1 << 19), MOD_ALT);
        assert_eq!(modifiers_from_ns_flags(1 << 17), MOD_SHIFT);
        assert_eq!(modifiers_from_ns_flags(1 << 20), MOD_WIN);
        assert_eq!(
            modifiers_from_ns_flags((1 << 19) | (1 << 20)),
            MOD_ALT | MOD_WIN
        );
    }

    #[test]
    fn the_flags_macos_sets_by_itself_are_not_modifiers() {
        // `fn` is set by the OS for every arrow and every F key, capsLock is a
        // latch, numericPad rides along with the keypad and the arrows.
        // Reading any of them would make ⌥⌘← a different combination from the
        // one the user pressed.
        for stray in [1 << 16, 1 << 21, 1 << 22, 1 << 23] {
            assert_eq!(modifiers_from_ns_flags(stray), 0, "flag {stray:#x}");
            assert_eq!(
                modifiers_from_ns_flags((1 << 20) | stray),
                MOD_WIN,
                "flag {stray:#x} next to Command"
            );
        }
    }

    #[test]
    fn carbon_modifiers_are_the_documented_bits() {
        assert_eq!(carbon_modifiers(0), 0);
        assert_eq!(carbon_modifiers(MOD_CTRL), 1 << 12);
        assert_eq!(carbon_modifiers(MOD_ALT), 1 << 11);
        assert_eq!(carbon_modifiers(MOD_SHIFT), 1 << 9);
        // Cmd, not Control: the parser aliases Win/Super/Meta/Cmd onto one bit
        // and this is the platform where that bit is Command.
        assert_eq!(carbon_modifiers(MOD_WIN), 1 << 8);
        assert_eq!(
            carbon_modifiers(MOD_CTRL | MOD_ALT | MOD_SHIFT | MOD_WIN),
            (1 << 12) | (1 << 11) | (1 << 9) | (1 << 8)
        );
    }

    // === classify_press ======================================================
    //
    // Hand-written key codes here, deliberately: this tier tests the function,
    // and the codes it is given are checked against recorded ones in
    // `tests/hotkey_replay.rs`, where the input came off a keyboard.

    #[test]
    fn a_modified_letter_is_bindable() {
        let verdict = classify_press(edge(0x26, MOD_ALT | MOD_WIN));
        assert_eq!(
            verdict,
            Verdict::Bindable(hotkey::parse("Alt+Cmd+J").unwrap())
        );
    }

    #[test]
    fn a_bare_letter_is_refused_by_name() {
        match classify_press(edge(0x26, 0)) {
            Verdict::NotBindable { name } => assert_eq!(name, "J"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn a_bare_f_key_records() {
        assert_eq!(
            classify_press(edge(0x69, 0)),
            Verdict::Bindable(hotkey::parse("F13").unwrap())
        );
    }

    #[test]
    fn bare_escape_cancels_and_a_modified_escape_records() {
        assert_eq!(classify_press(edge(0x35, 0)), Verdict::BareEscape);
        assert_eq!(
            classify_press(edge(0x35, MOD_WIN)),
            Verdict::Bindable(hotkey::parse("Cmd+Escape").unwrap())
        );
    }

    #[test]
    fn the_keypad_is_refused_by_name_and_is_not_the_digit_row() {
        // Two different keys, two different codes. The toolkit folded them
        // together and that fold is what the refusal has to survive.
        assert_ne!(keycode_for("5"), Some(0x57));
        match classify_press(edge(0x57, MOD_WIN)) {
            Verdict::NotBindable { name } => assert_eq!(name, "Keypad 5"),
            other => panic!("expected a refusal, got {other:?}"),
        }
        assert_eq!(
            classify_press(edge(keycode_for("5").unwrap(), MOD_WIN)),
            Verdict::Bindable(hotkey::parse("Cmd+5").unwrap())
        );
    }

    #[test]
    fn an_unknown_key_code_still_says_that_the_press_arrived() {
        match classify_press(edge(0xFF, MOD_WIN)) {
            Verdict::NotBindable { name } => assert!(name.contains("255"), "{name}"),
            other => panic!("expected a refusal, got {other:?}"),
        }
    }

    #[test]
    fn the_clipboard_chords_are_ordinary_bindable_keys_here() {
        // ⌘X / ⌘C / ⌘V only ever became clipboard commands because the toolkit
        // saw them first. At this layer they are three letters with Command
        // held, and there is no rule about them anywhere.
        for (code, label) in [(0x07, "Cmd+X"), (0x08, "Cmd+C"), (0x09, "Cmd+V")] {
            assert_eq!(
                classify_press(edge(code, MOD_WIN)),
                Verdict::Bindable(hotkey::parse(label).unwrap()),
                "{label}"
            );
        }
    }
}
