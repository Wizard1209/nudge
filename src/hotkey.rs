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
    /// "J" — a key that types something, with nothing held. The OS registers
    /// it and the key then belongs to us everywhere. §5.
    MissingModifier(String),
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
            ParseError::MissingModifier(k) => write!(
                f,
                "hotkey \"{}\" has no modifier — that would take the key away from every other application (only F1-F24 may stand alone)",
                k
            ),
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
    // §5: at least one modifier, except for the F-row. The registration call
    // accepts a bare key on both platforms and then owns it machine-wide —
    // this is the only place that says no.
    if modifiers == 0 && !is_function_key(&key) {
        return Err(ParseError::MissingModifier(key));
    }
    Ok(Hotkey {
        modifiers,
        key: HotkeyKey(key),
    })
}

/// Whether a canonical key token is one of `F1`..`F24` — the keys that type
/// nothing, and so are the only ones allowed to stand alone (§5).
fn is_function_key(key: &str) -> bool {
    key.strip_prefix('F')
        .and_then(|rest| rest.parse::<u8>().ok())
        .is_some_and(|n| (1..=24).contains(&n))
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
    fn a_key_that_types_something_needs_a_modifier() {
        // Registration would accept every one of these and then own the key
        // machine-wide (§5).
        for s in ["J", "5", "Space", "Enter", "/", "Left", "Delete"] {
            assert!(
                matches!(parse(s), Err(ParseError::MissingModifier(_))),
                "{s} must be refused without a modifier, got {:?}",
                parse(s)
            );
        }
    }

    #[test]
    fn the_f_row_may_stand_alone() {
        for s in ["F1", "F13", "F24"] {
            let hk = parse(s).unwrap_or_else(|e| panic!("{s} should parse: {e}"));
            assert_eq!(hk.modifiers, 0);
            assert_eq!(hk.key.as_str(), s);
        }
    }

    #[test]
    fn one_modifier_is_enough_for_anything() {
        for s in ["Ctrl+J", "⇧/", "Alt+Space"] {
            assert!(parse(s).is_ok(), "{s} should parse");
        }
    }

    #[test]
    fn default_hotkey_is_ctrl_shift_space() {
        let hk = default_hotkey();
        assert_eq!(format(&hk), "Ctrl+Shift+Space");
    }

}
