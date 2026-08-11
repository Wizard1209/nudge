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

/// The macOS symbol for each modifier, in the order Apple prints them in a
/// menu: Control, Option, Shift, Command. Our bit order is already that
/// order, so [`format`] and [`format_native`] walk the same list.
const MOD_GLYPH_CTRL: char = '⌃';
const MOD_GLYPH_ALT: char = '⌥';
const MOD_GLYPH_SHIFT: char = '⇧';
const MOD_GLYPH_WIN: char = '⌘';

/// The modifier bit a leading symbol stands for, or `None` if the character
/// is not a modifier symbol.
///
/// These are accepted on every platform, not just macOS. A user who pastes
/// `⌥⌘J` into the Windows build should get a parse error about the *key*
/// being unregistrable there, not "unknown token ⌥⌘J" — and the settings
/// window on macOS writes this form into the file, so a config copied
/// between machines has to keep meaning what it said.
fn modifier_glyph(c: char) -> Option<u8> {
    match c {
        MOD_GLYPH_CTRL => Some(MOD_CTRL),
        MOD_GLYPH_ALT => Some(MOD_ALT),
        MOD_GLYPH_SHIFT => Some(MOD_SHIFT),
        MOD_GLYPH_WIN | '⊞' => Some(MOD_WIN),
        _ => None,
    }
}

/// Parse a user-readable hotkey string like "Ctrl+Shift+Space" or "⌃⇧Space".
///
/// Rules:
/// - Case-insensitive; whitespace around `+` and around the whole string is ignored.
/// - Modifier aliases: `Control`/`Ctrl` and `⌃`; `Alt`/`Option`/`Opt` and `⌥`;
///   `Shift` and `⇧`; `Win`/`Super`/`Meta`/`Cmd`/`Command` and `⌘`.
/// - Modifier symbols need no `+` between them, because that is how macOS
///   writes a shortcut and the settings window puts exactly that string in
///   the field. `⌥⌘J`, `⌥+⌘+J` and `Alt+Cmd+J` are the same hotkey.
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
        let mut tok = segment.trim();
        if tok.is_empty() {
            // "Ctrl++A" or trailing "+" — treat as a malformed empty token.
            return Err(ParseError::UnknownToken(String::new()));
        }
        // Peel any leading modifier symbols off the segment. This is what
        // makes the separator optional: "⌥⌘J" arrives as one segment and
        // leaves "J" behind, while "Ctrl+⌘J" peels only in the second.
        while let Some(c) = tok.chars().next()
            && let Some(bit) = modifier_glyph(c)
        {
            modifiers |= bit;
            tok = tok[c.len_utf8()..].trim_start();
        }
        if tok.is_empty() {
            // The segment was symbols only — legal, unlike an empty one.
            continue;
        }
        let upper = tok.to_ascii_uppercase();
        match upper.as_str() {
            "CTRL" | "CONTROL" => modifiers |= MOD_CTRL,
            "ALT" | "OPTION" | "OPT" => modifiers |= MOD_ALT,
            "SHIFT" => modifiers |= MOD_SHIFT,
            "WIN" | "SUPER" | "META" | "CMD" | "COMMAND" => modifiers |= MOD_WIN,
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
/// The symbol caps, named by their unshifted legend (§5). One character each,
/// so they are their own canonical token and their own label on both
/// platforms. The shifted twin of a cap (`?` for `/`, `+` for `=`) is not a
/// spelling: it would be a second name for one binding, and `+` is the
/// separator besides.
const SYMBOL_KEYS: &str = "-=[]\\;',./`";

/// Every key that is not a letter, a digit or `F1`–`F24`, as
/// `(canonical token, accepted spellings, label, macOS glyph)`.
///
/// One row per physical key. The spellings are what §5 lists — including the
/// Mac symbols, so a shortcut written the way a Mac writes it round-trips.
/// The label is what [`format`] emits; the glyph is what [`format_native`]
/// shows on macOS, empty where the cap carries a word (`Space`) or has no cap
/// there at all (`Insert`).
#[rustfmt::skip]
const NAMED_KEYS: &[(&str, &[&str], &str, &str)] = &[
    ("SPACE",     &["SPACE", "␣"],                    "Space",     ""),
    ("ENTER",     &["ENTER", "RETURN", "↩", "⏎", "⌤"], "Enter",     "↩"),
    ("TAB",       &["TAB", "⇥"],                      "Tab",       "⇥"),
    ("ESCAPE",    &["ESCAPE", "ESC", "⎋"],            "Escape",    "⎋"),
    ("BACKSPACE", &["BACKSPACE", "⌫"],                "Backspace", "⌫"),
    ("LEFT",      &["LEFT", "←"],                     "Left",      "←"),
    ("RIGHT",     &["RIGHT", "→"],                    "Right",     "→"),
    ("UP",        &["UP", "↑"],                       "Up",        "↑"),
    ("DOWN",      &["DOWN", "↓"],                     "Down",      "↓"),
    ("HOME",      &["HOME", "↖"],                     "Home",      "↖"),
    ("END",       &["END", "↘"],                      "End",       "↘"),
    ("PAGEUP",    &["PAGEUP", "PGUP", "⇞"],           "PageUp",    "⇞"),
    ("PAGEDOWN",  &["PAGEDOWN", "PGDN", "⇟"],         "PageDown",  "⇟"),
    ("INSERT",    &["INSERT", "INS"],                 "Insert",    ""),
    ("DELETE",    &["DELETE", "DEL", "⌦"],            "Delete",    "⌦"),
];

fn canonicalize_key(upper: &str) -> Option<String> {
    if let Some(ch) = one_char(upper)
        && (ch.is_ascii_alphanumeric() || SYMBOL_KEYS.contains(ch))
    {
        return Some(upper.to_string());
    }
    // Function keys F1..F24
    if let Some(rest) = upper.strip_prefix('F')
        && let Ok(n) = rest.parse::<u8>()
        && (1..=24).contains(&n)
    {
        return Some(format!("F{}", n));
    }
    NAMED_KEYS
        .iter()
        .find(|(_, spellings, _, _)| spellings.contains(&upper))
        .map(|(token, ..)| token.to_string())
}

/// The single character of `s`, or `None` if it is not exactly one — by
/// character, not by byte, so a multi-byte glyph is one character and falls
/// through to [`NAMED_KEYS`] rather than being read as a symbol cap.
fn one_char(s: &str) -> Option<char> {
    let mut chars = s.chars();
    chars.next().filter(|_| chars.next().is_none())
}

/// How this platform spells the fourth modifier when writing a hotkey out.
/// The bit is one bit — `Win`, `Cmd` and `⌘` all parse to it everywhere — but
/// a config file a Mac user opens should say `Cmd`, not `Win`.
#[cfg(target_os = "macos")]
const MOD_WIN_LABEL: &str = "Cmd";
#[cfg(not(target_os = "macos"))]
const MOD_WIN_LABEL: &str = "Win";

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
        parts.push(MOD_WIN_LABEL);
    }
    let key_label = title_case_key(&hk.key.0);
    let mut out = parts.join("+");
    if !out.is_empty() {
        out.push('+');
    }
    out.push_str(&key_label);
    out
}

/// Every token a hotkey string may contain, in one sentence, spelled the way
/// this platform spells them.
///
/// This is the whole accepted vocabulary and it is deliberately one string
/// rather than a table: it is shown in the settings banner when a typed
/// hotkey does not parse, which is the only moment a user needs the list, and
/// a banner has room for a sentence.
///
/// The named keys are short because the backends are: Carbon's
/// `RegisterEventHotKey` and Win32's `RegisterHotKey` both take a single key
/// code, and this set is the intersection that maps on both. The keypad is
/// outside it on purpose — §5 says why.
// The modifier symbols stay — they are what the keycaps say, and the line
// exists to teach the form the field uses. The key symbols do not, with the
// arrows as the one exception: at the size this renders, `⌫` and `↩` are a
// smudge and have unambiguous names anyway, while `Left` reads as a word
// where `←` reads as the cap.
#[cfg(target_os = "macos")]
pub const VOCABULARY: &str = "Modifiers ⌃ ⌥ ⇧ ⌘ (or Ctrl, Option, Shift, Cmd) \
    plus one key: A–Z, 0–9, F1–F20, Space, Enter, Tab, Esc, Backspace, \
    ← ↑ → ↓, Home, End, PageUp, PageDown, Insert, Delete, \
    or a symbol cap: - = [ ] \\ ; ' , . / `";
#[cfg(not(target_os = "macos"))]
pub const VOCABULARY: &str = "Modifiers Ctrl, Alt, Shift, Win \
    plus one key: A–Z, 0–9, F1–F24, Space, Enter, Tab, Esc, Backspace, \
    ← ↑ → ↓, Home, End, PageUp, PageDown, Insert, Delete, \
    or a symbol cap: - = [ ] \\ ; ' , . / `";

/// The label to *show* a user of this platform — on macOS the symbol form,
/// `⌥⌘J`, which is what every other Mac app prints and what the keycaps say.
/// Everywhere else it is [`format`] unchanged, because `Ctrl+Alt+J` is what
/// Windows prints.
///
/// This is a rendering, not a second storage format: [`parse`] accepts what
/// this emits, so the settings window can put it straight in the editable
/// field and Save can hand it to the config unchanged.
///
/// The key half is symbolic only where a Mac keycap is: `⌘Space` and not
/// `⌘␣`, because the space bar has no legend and Apple's own menus write the
/// word.
pub fn format_native(hk: &Hotkey) -> String {
    #[cfg(not(target_os = "macos"))]
    {
        format(hk)
    }
    #[cfg(target_os = "macos")]
    {
        let mut out = String::with_capacity(8);
        for (bit, glyph) in [
            (MOD_CTRL, MOD_GLYPH_CTRL),
            (MOD_ALT, MOD_GLYPH_ALT),
            (MOD_SHIFT, MOD_GLYPH_SHIFT),
            (MOD_WIN, MOD_GLYPH_WIN),
        ] {
            if hk.modifiers & bit != 0 {
                out.push(glyph);
            }
        }
        let key_label = NAMED_KEYS
            .iter()
            .find(|(token, ..)| *token == hk.key.as_str())
            .map(|(_, _, _, glyph)| *glyph)
            .filter(|glyph| !glyph.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| title_case_key(hk.key.as_str()));
        out.push_str(&key_label);
        out
    }
}

fn title_case_key(upper: &str) -> String {
    // Letters, digits and symbol caps are one character and already read the
    // way they are written; F-keys stay uppercase. Everything else is a named
    // key, and [`NAMED_KEYS`] carries the spelling to show — `PageUp`, not the
    // `Pageup` a mechanical title-casing would produce.
    NAMED_KEYS
        .iter()
        .find(|(token, ..)| *token == upper)
        .map(|(_, _, label, _)| (*label).to_string())
        .unwrap_or_else(|| upper.to_string())
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
/// Returns `None` for any key outside that set (`fn`, the keypad, media keys,
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
        Key::ArrowLeft => "LEFT",
        Key::ArrowRight => "RIGHT",
        Key::ArrowUp => "UP",
        Key::ArrowDown => "DOWN",
        Key::Home => "HOME",
        Key::End => "END",
        Key::PageUp => "PAGEUP",
        Key::PageDown => "PAGEDOWN",
        Key::Insert => "INSERT",
        Key::Delete => "DELETE",
        // Symbol caps, named by their unshifted legend (§5). egui reports the
        // shifted twin as a variant of its own, so both fold onto the one
        // token. That is belt and braces behind `physical_key`, which already
        // reports the cap rather than what the layout printed — but this
        // function is also reached with a logical key, and then `⇧/` must not
        // become a second name for `/`.
        //
        // The digit row keeps only its digits: `!` is the sole shifted digit
        // egui names, so folding it onto `1` would be one exception where the
        // symbol caps are a rule.
        Key::Minus => "-",
        Key::Equals | Key::Plus => "=",
        Key::OpenBracket | Key::OpenCurlyBracket => "[",
        Key::CloseBracket | Key::CloseCurlyBracket => "]",
        Key::Backslash | Key::Pipe => "\\",
        Key::Semicolon | Key::Colon => ";",
        Key::Quote => "'",
        Key::Comma => ",",
        Key::Period => ".",
        Key::Slash | Key::Questionmark => "/",
        Key::Backtick => "`",
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

    // ---- the macOS vocabulary ------------------------------------------------

    #[test]
    fn modifier_symbols_parse_without_separators() {
        // This is the shape the settings field holds on macOS, so if it does
        // not parse, Save writes a hotkey the next launch cannot read.
        let hk = parse("⌥⌘J").unwrap();
        assert_eq!(hk.modifiers, MOD_ALT | MOD_WIN);
        assert_eq!(hk.key.as_str(), "J");
    }

    #[test]
    fn a_hotkey_is_one_hotkey_however_it_is_written() {
        // Symbols, symbols with separators, and words are three spellings of
        // one combination — the parser must not distinguish them.
        let canonical = parse("⌃⌥⇧⌘K").unwrap();
        for spelling in [
            "⌃+⌥+⇧+⌘+K",
            "Ctrl+Alt+Shift+Cmd+K",
            "Control+Option+Shift+Command+K",
            "ctrl+opt+shift+win+k",
            "⌘⇧+Ctrl+⌥K",
        ] {
            assert_eq!(
                parse(spelling).unwrap(),
                canonical,
                "spelling {spelling:?} must mean the same hotkey"
            );
        }
    }

    #[test]
    fn key_symbols_parse_to_the_same_keys_as_their_names() {
        for (symbol, name) in [
            ("⎋", "Escape"),
            ("⇥", "Tab"),
            ("⌫", "Backspace"),
            ("↩", "Enter"),
            ("␣", "Space"),
        ] {
            assert_eq!(
                parse(&format!("⌘{symbol}")).unwrap(),
                parse(&format!("Cmd+{name}")).unwrap(),
                "{symbol} must mean {name}"
            );
        }
    }

    #[test]
    fn a_symbol_only_string_still_needs_a_key() {
        // "⌥⌘" is a modifier chord, not a hotkey. Without this the peeling
        // loop would consume the whole string and leave no key to complain
        // about.
        assert_eq!(parse("⌥⌘").unwrap_err(), ParseError::MissingKey);
        assert_eq!(parse("⇧").unwrap_err(), ParseError::MissingKey);
    }

    #[test]
    fn an_unknown_key_after_symbols_is_still_reported_by_name() {
        // The error has to name the offending token, not the whole string —
        // the settings banner shows it verbatim.
        assert!(matches!(
            parse("⌘Numpad5"),
            Err(ParseError::UnknownToken(t)) if t == "Numpad5"
        ));
    }

    #[test]
    fn format_native_is_the_symbol_form_on_macos_and_the_word_form_elsewhere() {
        let hk = parse("Ctrl+Shift+Space").unwrap();
        #[cfg(target_os = "macos")]
        assert_eq!(format_native(&hk), "⌃⇧Space");
        #[cfg(not(target_os = "macos"))]
        assert_eq!(format_native(&hk), "Ctrl+Shift+Space");
    }

    #[test]
    fn format_native_roundtrips_through_parse() {
        // The settings field holds the native form and Save writes it to
        // config.json verbatim, so anything `format_native` can emit must
        // survive a round trip — otherwise a recorded hotkey is unreadable at
        // next launch.
        let samples = [
            "Ctrl+Shift+Space",
            "Alt+Cmd+J",
            "Ctrl+F12",
            "Cmd+Escape",
            "Shift+Backspace",
            "Ctrl+Alt+Shift+Cmd+7",
            "Cmd+Enter",
            "Alt+Tab",
        ];
        for s in samples {
            let hk = parse(s).unwrap();
            let shown = format_native(&hk);
            assert_eq!(
                parse(&shown).unwrap_or_else(|e| panic!("{shown:?} from {s:?} did not parse: {e}")),
                hk,
                "round trip failed for {s:?} → {shown:?}"
            );
        }
    }

    #[test]
    fn the_fourth_modifier_is_spelled_the_way_this_platform_spells_it() {
        let hk = parse("Cmd+J").unwrap();
        #[cfg(target_os = "macos")]
        assert_eq!(format(&hk), "Cmd+J");
        #[cfg(not(target_os = "macos"))]
        assert_eq!(format(&hk), "Win+J");
        // Whichever it printed, both spellings still parse back to it — a
        // config file carried between a Mac and a PC keeps its meaning.
        assert_eq!(parse(&format(&hk)).unwrap(), hk);
        assert_eq!(parse("Win+J").unwrap(), hk);
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
            Key::F25, // F25..F35 exist in egui but Win32 only goes F1..F24
            Key::Copy,
            Key::Cut,
            Key::Paste,
            Key::BrowserBack,
            // The one shifted digit egui names. The digit row is bound by its
            // digits, so folding this onto "1" would be an exception where the
            // symbol caps are a rule.
            Key::Exclamationmark,
        ] {
            assert!(
                hotkey_from_egui(Modifiers::CTRL, k).is_none(),
                "{k:?} must be rejected"
            );
        }
    }

    #[test]
    fn arrows_and_navigation_map() {
        let keys = [
            (Key::ArrowLeft, "LEFT"),
            (Key::ArrowRight, "RIGHT"),
            (Key::ArrowUp, "UP"),
            (Key::ArrowDown, "DOWN"),
            (Key::Home, "HOME"),
            (Key::End, "END"),
            (Key::PageUp, "PAGEUP"),
            (Key::PageDown, "PAGEDOWN"),
            (Key::Insert, "INSERT"),
            (Key::Delete, "DELETE"),
        ];
        for (k, expected) in keys {
            let hk = hotkey_from_egui(Modifiers::NONE, k).unwrap();
            assert_eq!(hk.key.as_str(), expected);
        }
    }

    #[test]
    fn a_symbol_cap_and_its_shifted_twin_are_one_key() {
        // egui reports `?` and `/` as different variants of one cap. Two
        // tokens would be two names for one binding — and one of them would
        // be unspellable, since `+` is the separator.
        let pairs = [
            (Key::Slash, Key::Questionmark, "/"),
            (Key::Backslash, Key::Pipe, "\\"),
            (Key::Equals, Key::Plus, "="),
            (Key::Semicolon, Key::Colon, ";"),
            (Key::OpenBracket, Key::OpenCurlyBracket, "["),
            (Key::CloseBracket, Key::CloseCurlyBracket, "]"),
        ];
        for (unshifted, shifted, expected) in pairs {
            for k in [unshifted, shifted] {
                let hk = hotkey_from_egui(Modifiers::NONE, k).unwrap();
                assert_eq!(hk.key.as_str(), expected, "{k:?}");
            }
        }
        for (k, expected) in [
            (Key::Minus, "-"),
            (Key::Quote, "'"),
            (Key::Comma, ","),
            (Key::Period, "."),
            (Key::Backtick, "`"),
        ] {
            let hk = hotkey_from_egui(Modifiers::NONE, k).unwrap();
            assert_eq!(hk.key.as_str(), expected);
        }
    }

    #[test]
    fn the_new_keys_round_trip_through_format_and_parse() {
        // Same contract as `roundtrip_via_format_and_parse`, extended over the
        // keys §5 grew: whatever the recorder produces, Save can write and the
        // next launch can read back.
        for k in [
            Key::ArrowLeft,
            Key::PageDown,
            Key::Delete,
            Key::Slash,
            Key::Minus,
            Key::Backtick,
            Key::Backslash,
        ] {
            let hk = hotkey_from_egui(Modifiers::ALT, k).expect("supported");
            let text = format(&hk);
            let parsed = parse(&text).expect("format() output must parse");
            assert_eq!(parsed, hk, "roundtrip failed for {text}");
        }
    }

    #[test]
    fn multi_word_named_keys_are_spelled_the_way_they_are_read() {
        // A mechanical title-casing would write "Pagedown" here, and that is
        // what goes into config.json and back out of it.
        assert_eq!(format(&parse("ctrl+pagedown").unwrap()), "Ctrl+PageDown");
        assert_eq!(format(&parse("ctrl+pgup").unwrap()), "Ctrl+PageUp");
        assert_eq!(format(&parse("ctrl+ins").unwrap()), "Ctrl+Insert");
    }

    #[test]
    fn symbol_caps_parse_in_both_spellings() {
        // The symbol form needs no separator after a modifier glyph, and the
        // word form needs one — the same rule as `⌥⌘J` versus `Alt+Cmd+J`.
        assert_eq!(parse("⌘,").unwrap(), parse("Cmd+,").unwrap());
        assert_eq!(parse("⌃⇧[").unwrap(), parse("Ctrl+Shift+[").unwrap());
        // `+` is the separator, never the key: the cap is `=`.
        assert_eq!(parse("Ctrl+=").unwrap().key.as_str(), "=");
        assert!(parse("Ctrl++").is_err());
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
