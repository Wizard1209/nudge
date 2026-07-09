//! Global hotkey: parsing user-readable strings ("Ctrl+Shift+Space") into a
//! modifier mask + key. The Win32 registration itself lives in
//! `tray_bridge`; everything in this module is platform-agnostic so it can
//! be unit-tested without pulling in `windows`.

use std::fmt;

pub const MOD_CTRL: u8 = 1 << 0;
pub const MOD_ALT: u8 = 1 << 1;
pub const MOD_SHIFT: u8 = 1 << 2;
pub const MOD_WIN: u8 = 1 << 3;

/// The "real" key part of a hotkey — everything that isn't a modifier.
/// Stored as the canonical, uppercased token (e.g. "A", "SPACE", "F5") so
/// round-tripping to a label is trivial.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyKey(String);

impl HotkeyKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotkey {
    pub modifiers: u8,
    pub key: HotkeyKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    /// "Ctrl+Shift" — nothing past the modifiers.
    MissingKey,
    /// "A+B" — two non-modifier tokens.
    MultipleKeys,
    /// "Foo+Ctrl" — segment is neither a modifier nor a known key.
    UnknownToken(String),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ParseError::Empty => write!(f, "hotkey string is empty"),
            ParseError::MissingKey => {
                write!(
                    f,
                    "hotkey has only modifiers, no key (e.g. need \"Ctrl+Shift+A\", not \"Ctrl+Shift\")"
                )
            }
            ParseError::MultipleKeys => {
                write!(f, "hotkey has more than one non-modifier key")
            }
            ParseError::UnknownToken(t) => write!(f, "unknown hotkey token: \"{}\"", t),
        }
    }
}

impl std::error::Error for ParseError {}

/// Parse a user-readable hotkey string like "Ctrl+Shift+Space".
///
/// Rules:
/// - Case-insensitive; whitespace around `+` and around the whole string is ignored.
/// - Modifier aliases: `Control`/`Ctrl`; `Alt`; `Shift`; `Win`/`Super`/`Meta`/`Cmd`.
/// - Exactly one non-modifier token is required.
/// - Duplicate modifiers ("Ctrl+Ctrl+A") are accepted silently — the user
///   probably copy-pasted; refusing would be more annoying than helpful.
pub fn parse(input: &str) -> Result<Hotkey, ParseError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(ParseError::Empty);
    }

    let mut modifiers: u8 = 0;
    let mut key: Option<String> = None;

    for segment in trimmed.split('+') {
        let tok = segment.trim();
        if tok.is_empty() {
            // "Ctrl++A" or trailing "+" — treat as a malformed empty token.
            return Err(ParseError::UnknownToken(String::new()));
        }
        let upper = tok.to_ascii_uppercase();
        match upper.as_str() {
            "CTRL" | "CONTROL" => modifiers |= MOD_CTRL,
            "ALT" => modifiers |= MOD_ALT,
            "SHIFT" => modifiers |= MOD_SHIFT,
            "WIN" | "SUPER" | "META" | "CMD" => modifiers |= MOD_WIN,
            _ => {
                let canonical = canonicalize_key(&upper)
                    .ok_or_else(|| ParseError::UnknownToken(tok.to_string()))?;
                if key.is_some() {
                    return Err(ParseError::MultipleKeys);
                }
                key = Some(canonical);
            }
        }
    }

    let key = key.ok_or(ParseError::MissingKey)?;
    Ok(Hotkey {
        modifiers,
        key: HotkeyKey(key),
    })
}

/// Return the canonical form of a key token (already uppercased) if we
/// recognise it. None means "not a key we know how to register". Kept
/// separate from `parse` so the platform layer can also use it for
/// VK-mapping without re-parsing the whole string.
fn canonicalize_key(upper: &str) -> Option<String> {
    if upper.len() == 1 {
        let ch = upper.chars().next().unwrap();
        if ch.is_ascii_alphanumeric() {
            return Some(upper.to_string());
        }
    }
    // Function keys F1..F24
    if let Some(rest) = upper.strip_prefix('F')
        && let Ok(n) = rest.parse::<u8>()
        && (1..=24).contains(&n)
    {
        return Some(format!("F{}", n));
    }
    // Named keys we're willing to register globally. Deliberately conservative:
    // home/end/arrows/etc. can stay un-supported until someone asks. Most users
    // pick Letter+modifiers or Space for spotlight-style shortcuts.
    match upper {
        "SPACE" | "ENTER" | "RETURN" | "TAB" | "ESC" | "ESCAPE" | "BACKSPACE" => {
            // Normalize aliases.
            Some(match upper {
                "RETURN" => "ENTER".to_string(),
                "ESC" => "ESCAPE".to_string(),
                other => other.to_string(),
            })
        }
        _ => None,
    }
}

/// Reconstruct the canonical label for a Hotkey ("Ctrl+Shift+Space"). Used
/// when emitting the default config so the file looks the way the docs
/// describe it.
pub fn format(hk: &Hotkey) -> String {
    let mut parts: Vec<&str> = Vec::with_capacity(5);
    if hk.modifiers & MOD_CTRL != 0 {
        parts.push("Ctrl");
    }
    if hk.modifiers & MOD_ALT != 0 {
        parts.push("Alt");
    }
    if hk.modifiers & MOD_SHIFT != 0 {
        parts.push("Shift");
    }
    if hk.modifiers & MOD_WIN != 0 {
        parts.push("Win");
    }
    let key_label = title_case_key(&hk.key.0);
    let mut out = parts.join("+");
    if !out.is_empty() {
        out.push('+');
    }
    out.push_str(&key_label);
    out
}

fn title_case_key(upper: &str) -> String {
    // F-keys stay uppercase. Single letters stay uppercase. Multi-char names
    // (Space, Enter, Escape, …) get title-cased so the rendered label reads
    // naturally instead of shouting.
    let is_f_key = upper.starts_with('F') && upper[1..].chars().all(|c| c.is_ascii_digit());
    if upper.len() == 1 || is_f_key {
        return upper.to_string();
    }
    let mut chars = upper.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => {
            let rest: String = chars.as_str().to_ascii_lowercase();
            format!("{}{}", first, rest)
        }
    }
}

/// Default hotkey used when no config file exists yet. Ctrl+Shift+Space is
/// rare enough on Windows to be free in most setups (no system binding), and
/// memorable as "spotlight-ish".
pub fn default_hotkey() -> Hotkey {
    parse("Ctrl+Shift+Space").expect("default hotkey must parse")
}

/// Translate an egui-side `(modifiers, key)` pair from the settings recorder
/// into a `Hotkey` in our canonical form. The supported key set deliberately
/// mirrors what [`canonicalize_key`] accepts and what
/// [`crate::tray_bridge::vk_for_key`] can translate to a Win32 VK — so the
/// recorder cannot produce a label that would silently fail to register at
/// next launch.
///
/// Returns `None` for any key outside that set (arrows, Home/End, punctuation,
/// dead keys, …) — the caller surfaces a hint and keeps recording.
///
/// On the modifier side, egui's `mac_cmd` / `command` both fold into our
/// `MOD_WIN` bit, matching the parser's `Win|Super|Meta|Cmd` aliasing rule.
/// Callers must pass a real non-modifier key as `key`; egui doesn't surface
/// raw Ctrl/Alt/Shift as `Key` variants, so this contract is naturally upheld
/// by the input-pump loop.
//
// Bin-on-wasm reports this as "never used" because main.rs's settings_app
// import is gated to non-wasm — same noise the rest of this module already
// emits in that build. The library and the native bin both use this fn.
#[allow(dead_code)]
pub fn hotkey_from_egui(
    modifiers: eframe::egui::Modifiers,
    key: eframe::egui::Key,
) -> Option<Hotkey> {
    use eframe::egui::Key;

    let token: &'static str = match key {
        Key::A => "A",
        Key::B => "B",
        Key::C => "C",
        Key::D => "D",
        Key::E => "E",
        Key::F => "F",
        Key::G => "G",
        Key::H => "H",
        Key::I => "I",
        Key::J => "J",
        Key::K => "K",
        Key::L => "L",
        Key::M => "M",
        Key::N => "N",
        Key::O => "O",
        Key::P => "P",
        Key::Q => "Q",
        Key::R => "R",
        Key::S => "S",
        Key::T => "T",
        Key::U => "U",
        Key::V => "V",
        Key::W => "W",
        Key::X => "X",
        Key::Y => "Y",
        Key::Z => "Z",
        Key::Num0 => "0",
        Key::Num1 => "1",
        Key::Num2 => "2",
        Key::Num3 => "3",
        Key::Num4 => "4",
        Key::Num5 => "5",
        Key::Num6 => "6",
        Key::Num7 => "7",
        Key::Num8 => "8",
        Key::Num9 => "9",
        Key::F1 => "F1",
        Key::F2 => "F2",
        Key::F3 => "F3",
        Key::F4 => "F4",
        Key::F5 => "F5",
        Key::F6 => "F6",
        Key::F7 => "F7",
        Key::F8 => "F8",
        Key::F9 => "F9",
        Key::F10 => "F10",
        Key::F11 => "F11",
        Key::F12 => "F12",
        Key::F13 => "F13",
        Key::F14 => "F14",
        Key::F15 => "F15",
        Key::F16 => "F16",
        Key::F17 => "F17",
        Key::F18 => "F18",
        Key::F19 => "F19",
        Key::F20 => "F20",
        Key::F21 => "F21",
        Key::F22 => "F22",
        Key::F23 => "F23",
        Key::F24 => "F24",
        Key::Space => "SPACE",
        Key::Enter => "ENTER",
        Key::Tab => "TAB",
        Key::Escape => "ESCAPE",
        Key::Backspace => "BACKSPACE",
        _ => return None,
    };

    let mut mods: u8 = 0;
    if modifiers.ctrl {
        mods |= MOD_CTRL;
    }
    if modifiers.alt {
        mods |= MOD_ALT;
    }
    if modifiers.shift {
        mods |= MOD_SHIFT;
    }
    // egui's `command` is the "primary modifier": Cmd on mac, Ctrl on others.
    // `mac_cmd` is the raw Cmd key (mac only). Both fold into our MOD_WIN bit
    // for cross-platform consistency with the parser's Win|Super|Meta|Cmd
    // aliasing — except on non-mac targets `command == ctrl`, which we already
    // handled above, so we only add MOD_WIN when `mac_cmd` is set (true Cmd).
    if modifiers.mac_cmd {
        mods |= MOD_WIN;
    }

    Some(Hotkey {
        modifiers: mods,
        key: HotkeyKey(token.to_string()),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ctrl_shift_space() {
        let hk = parse("Ctrl+Shift+Space").unwrap();
        assert_eq!(hk.modifiers, MOD_CTRL | MOD_SHIFT);
        assert_eq!(hk.key.as_str(), "SPACE");
    }

    #[test]
    fn parses_case_insensitively() {
        let lower = parse("ctrl+shift+space").unwrap();
        let upper = parse("CTRL+SHIFT+SPACE").unwrap();
        let mixed = parse("Ctrl+Shift+Space").unwrap();
        assert_eq!(lower, upper);
        assert_eq!(lower, mixed);
    }

    #[test]
    fn tolerates_whitespace() {
        let hk = parse("  Ctrl +  Shift +Space  ").unwrap();
        assert_eq!(hk.modifiers, MOD_CTRL | MOD_SHIFT);
        assert_eq!(hk.key.as_str(), "SPACE");
    }

    #[test]
    fn accepts_control_alias() {
        let hk = parse("Control+A").unwrap();
        assert_eq!(hk.modifiers, MOD_CTRL);
        assert_eq!(hk.key.as_str(), "A");
    }

    #[test]
    fn accepts_win_super_meta_cmd_as_win() {
        for s in ["Win+J", "Super+J", "Meta+J", "Cmd+J"] {
            let hk = parse(s).unwrap();
            assert_eq!(hk.modifiers, MOD_WIN, "for {}", s);
            assert_eq!(hk.key.as_str(), "J", "for {}", s);
        }
    }

    #[test]
    fn single_letter_key_is_recognised() {
        let hk = parse("Ctrl+A").unwrap();
        assert_eq!(hk.modifiers, MOD_CTRL);
        assert_eq!(hk.key.as_str(), "A");
    }

    #[test]
    fn digit_key_is_recognised() {
        let hk = parse("Ctrl+0").unwrap();
        assert_eq!(hk.key.as_str(), "0");
    }

    #[test]
    fn f_keys_recognised() {
        for n in 1..=12u8 {
            let hk = parse(&format!("Ctrl+F{}", n)).unwrap();
            assert_eq!(hk.key.as_str(), &format!("F{}", n));
        }
    }

    #[test]
    fn f25_is_rejected() {
        // F1..F24 are valid VK codes; F25 is not.
        assert!(matches!(
            parse("Ctrl+F25"),
            Err(ParseError::UnknownToken(_))
        ));
    }

    #[test]
    fn named_keys() {
        assert_eq!(parse("Ctrl+Enter").unwrap().key.as_str(), "ENTER");
        assert_eq!(parse("Ctrl+Return").unwrap().key.as_str(), "ENTER");
        assert_eq!(parse("Ctrl+Esc").unwrap().key.as_str(), "ESCAPE");
        assert_eq!(parse("Ctrl+Tab").unwrap().key.as_str(), "TAB");
    }

    #[test]
    fn duplicate_modifiers_collapse() {
        let hk = parse("Ctrl+Ctrl+A").unwrap();
        assert_eq!(hk.modifiers, MOD_CTRL);
    }

    #[test]
    fn empty_input_errors() {
        assert_eq!(parse("").unwrap_err(), ParseError::Empty);
        assert_eq!(parse("   ").unwrap_err(), ParseError::Empty);
    }

    #[test]
    fn only_modifiers_errors() {
        assert_eq!(parse("Ctrl+Shift").unwrap_err(), ParseError::MissingKey);
        assert_eq!(parse("Ctrl").unwrap_err(), ParseError::MissingKey);
    }

    #[test]
    fn two_keys_errors() {
        assert_eq!(parse("Ctrl+A+B").unwrap_err(), ParseError::MultipleKeys);
    }

    #[test]
    fn unknown_token_errors() {
        assert!(matches!(parse("Foo+Ctrl+A"), Err(ParseError::UnknownToken(t)) if t == "Foo"));
    }

    #[test]
    fn empty_segment_errors() {
        // "Ctrl++A" — adjacent + characters mean a missing token.
        assert!(matches!(parse("Ctrl++A"), Err(ParseError::UnknownToken(_))));
    }

    #[test]
    fn format_roundtrips_ctrl_shift_space() {
        let hk = parse("Ctrl+Shift+Space").unwrap();
        assert_eq!(format(&hk), "Ctrl+Shift+Space");
    }

    #[test]
    fn format_orders_modifiers_canonically() {
        // Whatever the user types, format() always emits in the same order:
        // Ctrl, Alt, Shift, Win. Keeps the rendered default and round-tripped
        // strings predictable for diffing.
        let hk = parse("Shift+Ctrl+A").unwrap();
        assert_eq!(format(&hk), "Ctrl+Shift+A");
    }

    #[test]
    fn format_keeps_f_keys_uppercase() {
        let hk = parse("ctrl+f5").unwrap();
        assert_eq!(format(&hk), "Ctrl+F5");
    }

    #[test]
    fn default_hotkey_is_ctrl_shift_space() {
        let hk = default_hotkey();
        assert_eq!(format(&hk), "Ctrl+Shift+Space");
    }

    // ---- hotkey_from_egui ---------------------------------------------------
    // The recorder's pure mapping function. Verifies the supported key set
    // exactly matches what `canonicalize_key` (and therefore
    // `tray_bridge::vk_for_key`) accepts, so a recorded combo CAN be
    // registered on Windows at next launch.

    use eframe::egui::{Key, Modifiers};

    #[test]
    fn letters_map_to_uppercase_token() {
        // egui::Key::A..=Z → "A".."Z". Modifier-less recording produces a
        // bare-letter hotkey; the parser allows it (Cmd+A etc. are common,
        // but bare-A is also legal — just impractical).
        let letters = [
            (Key::A, "A"),
            (Key::B, "B"),
            (Key::C, "C"),
            (Key::D, "D"),
            (Key::E, "E"),
            (Key::F, "F"),
            (Key::G, "G"),
            (Key::H, "H"),
            (Key::I, "I"),
            (Key::J, "J"),
            (Key::K, "K"),
            (Key::L, "L"),
            (Key::M, "M"),
            (Key::N, "N"),
            (Key::O, "O"),
            (Key::P, "P"),
            (Key::Q, "Q"),
            (Key::R, "R"),
            (Key::S, "S"),
            (Key::T, "T"),
            (Key::U, "U"),
            (Key::V, "V"),
            (Key::W, "W"),
            (Key::X, "X"),
            (Key::Y, "Y"),
            (Key::Z, "Z"),
        ];
        for (k, expected) in letters {
            let hk = hotkey_from_egui(Modifiers::NONE, k)
                .unwrap_or_else(|| panic!("letter {expected:?} must map"));
            assert_eq!(hk.key.as_str(), expected, "letter {expected:?}");
            assert_eq!(hk.modifiers, 0, "no modifiers passed");
        }
    }

    #[test]
    fn digits_map_to_token() {
        let digits = [
            (Key::Num0, "0"),
            (Key::Num1, "1"),
            (Key::Num2, "2"),
            (Key::Num3, "3"),
            (Key::Num4, "4"),
            (Key::Num5, "5"),
            (Key::Num6, "6"),
            (Key::Num7, "7"),
            (Key::Num8, "8"),
            (Key::Num9, "9"),
        ];
        for (k, expected) in digits {
            let hk = hotkey_from_egui(Modifiers::CTRL, k).unwrap();
            assert_eq!(hk.key.as_str(), expected);
            assert_eq!(hk.modifiers, MOD_CTRL);
        }
    }

    #[test]
    fn f_keys_map_to_token() {
        // F1..F24 — the same range vk_for_key registers globally.
        let f_keys = [
            (Key::F1, "F1"),
            (Key::F2, "F2"),
            (Key::F3, "F3"),
            (Key::F4, "F4"),
            (Key::F5, "F5"),
            (Key::F6, "F6"),
            (Key::F7, "F7"),
            (Key::F8, "F8"),
            (Key::F9, "F9"),
            (Key::F10, "F10"),
            (Key::F11, "F11"),
            (Key::F12, "F12"),
            (Key::F13, "F13"),
            (Key::F14, "F14"),
            (Key::F15, "F15"),
            (Key::F16, "F16"),
            (Key::F17, "F17"),
            (Key::F18, "F18"),
            (Key::F19, "F19"),
            (Key::F20, "F20"),
            (Key::F21, "F21"),
            (Key::F22, "F22"),
            (Key::F23, "F23"),
            (Key::F24, "F24"),
        ];
        for (k, expected) in f_keys {
            let hk = hotkey_from_egui(Modifiers::NONE, k).unwrap();
            assert_eq!(hk.key.as_str(), expected);
        }
    }

    #[test]
    fn named_keys_map() {
        // Same allowlist canonicalize_key permits — Space, Enter, Tab, Escape,
        // Backspace. Aliases (Return → ENTER, Esc → ESCAPE) don't appear in
        // egui::Key, so no aliasing logic needed here.
        let named = [
            (Key::Space, "SPACE"),
            (Key::Enter, "ENTER"),
            (Key::Tab, "TAB"),
            (Key::Escape, "ESCAPE"),
            (Key::Backspace, "BACKSPACE"),
        ];
        for (k, expected) in named {
            let hk = hotkey_from_egui(Modifiers::NONE, k).unwrap();
            assert_eq!(hk.key.as_str(), expected);
        }
    }

    #[test]
    fn modifiers_compose() {
        // Ctrl+Shift+A → both bits set.
        let mods = Modifiers {
            ctrl: true,
            shift: true,
            ..Modifiers::NONE
        };
        let hk = hotkey_from_egui(mods, Key::A).unwrap();
        assert_eq!(hk.modifiers, MOD_CTRL | MOD_SHIFT);
        assert_eq!(hk.key.as_str(), "A");
    }

    #[test]
    fn all_four_modifiers_compose() {
        // Ctrl+Alt+Shift+Cmd+J — all four bits set. Tests that mac_cmd folds
        // into our MOD_WIN bit (the parser treats Cmd as an alias for Win).
        let mods = Modifiers {
            ctrl: true,
            alt: true,
            shift: true,
            mac_cmd: true,
            command: true,
        };
        let hk = hotkey_from_egui(mods, Key::J).unwrap();
        assert_eq!(hk.modifiers, MOD_CTRL | MOD_ALT | MOD_SHIFT | MOD_WIN);
        assert_eq!(hk.key.as_str(), "J");
    }

    #[test]
    fn unsupported_key_returns_none() {
        // Anything outside the canonicalize_key allowlist must be rejected —
        // otherwise the recorder writes a label that fails to register at
        // next launch, silently.
        for k in [
            Key::Home,
            Key::End,
            Key::Insert,
            Key::Delete,
            Key::PageUp,
            Key::PageDown,
            Key::ArrowUp,
            Key::ArrowDown,
            Key::ArrowLeft,
            Key::ArrowRight,
            Key::Comma,
            Key::Period,
            Key::Semicolon,
            Key::Quote,
            Key::Minus,
            Key::Plus,
            Key::Equals,
            Key::Slash,
            Key::Backslash,
            Key::F25, // F25..F35 exist in egui but Win32 only goes F1..F24
        ] {
            assert!(
                hotkey_from_egui(Modifiers::CTRL, k).is_none(),
                "{k:?} must be rejected"
            );
        }
    }

    #[test]
    fn roundtrip_via_format_and_parse() {
        // Whatever the recorder produces, format() → parse() must reconstruct
        // the same Hotkey. This is the contract that lets Save just write the
        // text label: the parser will accept it back on next launch.
        let samples: &[(Modifiers, Key)] = &[
            (
                Modifiers {
                    ctrl: true,
                    shift: true,
                    ..Modifiers::NONE
                },
                Key::Space,
            ),
            (Modifiers::ALT, Key::J),
            (Modifiers::NONE, Key::F12),
            (
                Modifiers {
                    ctrl: true,
                    alt: true,
                    ..Modifiers::NONE
                },
                Key::Num7,
            ),
            (Modifiers::CTRL, Key::Enter),
            (Modifiers::CTRL, Key::Backspace),
        ];
        for (m, k) in samples {
            let hk = hotkey_from_egui(*m, *k).expect("supported");
            let text = format(&hk);
            let parsed = parse(&text).expect("format() output must parse");
            assert_eq!(parsed, hk, "roundtrip failed for {text}");
        }
    }
}
