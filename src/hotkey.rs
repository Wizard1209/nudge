use std::fmt;
use std::ops::{BitOr, BitOrAssign};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash)]
pub struct Modifiers(u8);

impl Modifiers {
    pub const NONE: Self = Self(0);
    pub const CTRL: Self = Self(1 << 0);
    pub const ALT: Self = Self(1 << 1);
    pub const SHIFT: Self = Self(1 << 2);
    pub const WIN: Self = Self(1 << 3);

    pub const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub const fn bits(self) -> u8 {
        self.0
    }

    pub const fn from_bits(bits: u8) -> Option<Self> {
        let all = Self::CTRL.0 | Self::ALT.0 | Self::SHIFT.0 | Self::WIN.0;
        if bits & !all == 0 {
            Some(Self(bits))
        } else {
            None
        }
    }
}

impl BitOr for Modifiers {
    type Output = Self;
    fn bitor(self, rhs: Self) -> Self {
        Self(self.0 | rhs.0)
    }
}

impl BitOrAssign for Modifiers {
    fn bitor_assign(&mut self, rhs: Self) {
        self.0 |= rhs.0;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HotkeyKey(String);

impl HotkeyKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hotkey {
    pub modifiers: Modifiers,
    pub key: HotkeyKey,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ParseError {
    Empty,
    MissingKey,
    MultipleKeys,
    UnknownToken(String),
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
            ParseError::UnknownToken(t) => write!(f, "unknown hotkey token: \"{}\"", echoed(t)),
            ParseError::MissingModifier(k) => write!(
                f,
                "hotkey \"{}\" has no modifier — that would take the key away from every other application (only F1-F24 may stand alone)",
                k
            ),
        }
    }
}

impl std::error::Error for ParseError {}

pub const ECHO_LIMIT: usize = 40;

pub fn echoed(text: &str) -> String {
    let mut chars = text.chars();
    let head: String = chars.by_ref().take(ECHO_LIMIT).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

const MOD_GLYPH_CTRL: char = '⌃';
const MOD_GLYPH_ALT: char = '⌥';
const MOD_GLYPH_SHIFT: char = '⇧';
const MOD_GLYPH_WIN: char = '⌘';

fn modifier_glyph(c: char) -> Option<Modifiers> {
    match c {
        MOD_GLYPH_CTRL => Some(Modifiers::CTRL),
        MOD_GLYPH_ALT => Some(Modifiers::ALT),
        MOD_GLYPH_SHIFT => Some(Modifiers::SHIFT),
        MOD_GLYPH_WIN | '⊞' => Some(Modifiers::WIN),
        _ => None,
    }
}

pub fn parse(input: &str) -> Result<Hotkey, ParseError> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(ParseError::Empty);
    }

    let mut modifiers = Modifiers::NONE;
    let mut key: Option<&str> = None;

    for segment in trimmed.split('+') {
        let mut tok = segment.trim();
        if tok.is_empty() {
            return Err(ParseError::UnknownToken(String::new()));
        }
        while let Some(c) = tok.chars().next()
            && let Some(bit) = modifier_glyph(c)
        {
            modifiers |= bit;
            tok = tok[c.len_utf8()..].trim_start();
        }
        if tok.is_empty() {
            continue;
        }
        let upper = tok.to_ascii_uppercase();
        match upper.as_str() {
            "CTRL" | "CONTROL" => modifiers |= Modifiers::CTRL,
            "ALT" | "OPTION" | "OPT" => modifiers |= Modifiers::ALT,
            "SHIFT" => modifiers |= Modifiers::SHIFT,
            "WIN" | "SUPER" | "META" | "CMD" | "COMMAND" => modifiers |= Modifiers::WIN,
            _ => {
                if canonicalize_key(&upper).is_none() {
                    return Err(ParseError::UnknownToken(tok.to_string()));
                }
                if key.is_some() {
                    return Err(ParseError::MultipleKeys);
                }
                key = Some(tok);
            }
        }
    }

    let key = key.ok_or(ParseError::MissingKey)?;
    from_parts(modifiers, key)
}

pub fn from_parts(modifiers: Modifiers, token: &str) -> Result<Hotkey, ParseError> {
    let upper = token.to_ascii_uppercase();
    let key =
        canonicalize_key(&upper).ok_or_else(|| ParseError::UnknownToken(token.to_string()))?;
    if modifiers.is_empty() && !is_function_key(&key) {
        return Err(ParseError::MissingModifier(key));
    }
    Ok(Hotkey {
        modifiers,
        key: HotkeyKey(key),
    })
}

pub fn all_key_tokens() -> Vec<String> {
    let mut out: Vec<String> = Vec::with_capacity(80);
    for ch in 'A'..='Z' {
        out.push(ch.to_string());
    }
    for ch in '0'..='9' {
        out.push(ch.to_string());
    }
    for n in 1..=MAX_FUNCTION_KEY {
        out.push(format!("F{n}"));
    }
    for key in NAMED_KEYS {
        out.push(key.token.to_string());
    }
    for ch in SYMBOL_KEYS.chars() {
        out.push(ch.to_string());
    }
    out
}

const MAX_FUNCTION_KEY: u8 = 24;

pub fn function_key_number(upper: &str) -> Option<u8> {
    upper
        .strip_prefix('F')
        .and_then(|rest| rest.parse::<u8>().ok())
        .filter(|n| (1..=MAX_FUNCTION_KEY).contains(n))
}

fn is_function_key(key: &str) -> bool {
    function_key_number(key).is_some()
}

const SYMBOL_KEYS: &str = "-=[]\\;',./`";

struct NamedKey {
    token: &'static str,
    spellings: &'static [&'static str],
    label: &'static str,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    glyph: &'static str,
}

const fn named(
    token: &'static str,
    spellings: &'static [&'static str],
    label: &'static str,
    glyph: &'static str,
) -> NamedKey {
    NamedKey {
        token,
        spellings,
        label,
        glyph,
    }
}

#[rustfmt::skip]
const NAMED_KEYS: &[NamedKey] = &[
    named("SPACE",     &["SPACE", "␣"],                    "Space",     ""),
    named("ENTER",     &["ENTER", "RETURN", "↩", "⏎", "⌤"], "Enter",     "↩"),
    named("TAB",       &["TAB", "⇥"],                      "Tab",       "⇥"),
    named("ESCAPE",    &["ESCAPE", "ESC", "⎋"],            "Escape",    "⎋"),
    named("BACKSPACE", &["BACKSPACE", "⌫"],                "Backspace", "⌫"),
    named("LEFT",      &["LEFT", "←"],                     "Left",      "←"),
    named("RIGHT",     &["RIGHT", "→"],                    "Right",     "→"),
    named("UP",        &["UP", "↑"],                       "Up",        "↑"),
    named("DOWN",      &["DOWN", "↓"],                     "Down",      "↓"),
    named("HOME",      &["HOME", "↖"],                     "Home",      "↖"),
    named("END",       &["END", "↘"],                      "End",       "↘"),
    named("PAGEUP",    &["PAGEUP", "PGUP", "⇞"],           "PageUp",    "⇞"),
    named("PAGEDOWN",  &["PAGEDOWN", "PGDN", "⇟"],         "PageDown",  "⇟"),
    named("INSERT",    &["INSERT", "INS"],                 "Insert",    ""),
    named("DELETE",    &["DELETE", "DEL", "⌦"],            "Delete",    "⌦"),
];

fn canonicalize_key(upper: &str) -> Option<String> {
    if let Some(ch) = one_char(upper)
        && (ch.is_ascii_alphanumeric() || SYMBOL_KEYS.contains(ch))
    {
        return Some(upper.to_string());
    }
    if let Some(n) = function_key_number(upper) {
        return Some(format!("F{n}"));
    }
    NAMED_KEYS
        .iter()
        .find(|key| key.spellings.contains(&upper))
        .map(|key| key.token.to_string())
}

fn one_char(s: &str) -> Option<char> {
    let mut chars = s.chars();
    chars.next().filter(|_| chars.next().is_none())
}

#[cfg(target_os = "macos")]
const MOD_WIN_LABEL: &str = "Cmd";
#[cfg(not(target_os = "macos"))]
const MOD_WIN_LABEL: &str = "Win";

pub fn format(hk: &Hotkey) -> String {
    let mut parts: Vec<&str> = Vec::with_capacity(5);
    if hk.modifiers.contains(Modifiers::CTRL) {
        parts.push("Ctrl");
    }
    if hk.modifiers.contains(Modifiers::ALT) {
        parts.push("Alt");
    }
    if hk.modifiers.contains(Modifiers::SHIFT) {
        parts.push("Shift");
    }
    if hk.modifiers.contains(Modifiers::WIN) {
        parts.push(MOD_WIN_LABEL);
    }
    let key_label = key_label(&hk.key.0);
    let mut out = parts.join("+");
    if !out.is_empty() {
        out.push('+');
    }
    out.push_str(&key_label);
    out
}

#[cfg(target_os = "macos")]
pub const VOCABULARY: &str = "Modifiers ⌃ ⌥ ⇧ ⌘ (or Ctrl, Option, Shift, Cmd) \
    plus one key: A–Z, 0–9, F1–F20, Space, Enter, Tab, Esc, Backspace, \
    ← ↑ → ↓, Home, End, PageUp, PageDown, Insert, Delete, \
    or a symbol cap: - = [ ] \\ ; ' , . / `";
#[cfg(not(target_os = "macos"))]
pub const VOCABULARY: &str = "Modifiers Ctrl, Alt, Shift, Win \
    plus one key: A–Z, 0–9, F1–F24, Space, Enter, Tab, Esc, Backspace, \
    Left, Right, Up, Down, Home, End, PageUp, PageDown, Insert, Delete, \
    or a symbol cap: - = [ ] \\ ; ' , . / `";

pub fn format_native(hk: &Hotkey) -> String {
    #[cfg(not(target_os = "macos"))]
    {
        format(hk)
    }
    #[cfg(target_os = "macos")]
    {
        let mut out = String::with_capacity(8);
        for (bit, glyph) in [
            (Modifiers::CTRL, MOD_GLYPH_CTRL),
            (Modifiers::ALT, MOD_GLYPH_ALT),
            (Modifiers::SHIFT, MOD_GLYPH_SHIFT),
            (Modifiers::WIN, MOD_GLYPH_WIN),
        ] {
            if hk.modifiers.contains(bit) {
                out.push(glyph);
            }
        }
        let key_label = NAMED_KEYS
            .iter()
            .find(|key| key.token == hk.key.as_str())
            .map(|key| key.glyph)
            .filter(|glyph| !glyph.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| key_label(hk.key.as_str()));
        out.push_str(&key_label);
        out
    }
}

pub fn key_label(token: &str) -> String {
    NAMED_KEYS
        .iter()
        .find(|key| key.token == token)
        .map(|key| key.label.to_string())
        .unwrap_or_else(|| token.to_string())
}

pub fn default_hotkey() -> Hotkey {
    parse("Ctrl+Shift+Space").expect("default hotkey must parse")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_ctrl_shift_space() {
        let hk = parse("Ctrl+Shift+Space").unwrap();
        assert_eq!(hk.modifiers, Modifiers::CTRL | Modifiers::SHIFT);
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
        assert_eq!(hk.modifiers, Modifiers::CTRL | Modifiers::SHIFT);
        assert_eq!(hk.key.as_str(), "SPACE");
    }

    #[test]
    fn accepts_control_alias() {
        let hk = parse("Control+A").unwrap();
        assert_eq!(hk.modifiers, Modifiers::CTRL);
        assert_eq!(hk.key.as_str(), "A");
    }

    #[test]
    fn accepts_win_super_meta_cmd_as_win() {
        for s in ["Win+J", "Super+J", "Meta+J", "Cmd+J"] {
            let hk = parse(s).unwrap();
            assert_eq!(hk.modifiers, Modifiers::WIN, "for {}", s);
            assert_eq!(hk.key.as_str(), "J", "for {}", s);
        }
    }

    #[test]
    fn single_letter_key_is_recognised() {
        let hk = parse("Ctrl+A").unwrap();
        assert_eq!(hk.modifiers, Modifiers::CTRL);
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
        assert_eq!(hk.modifiers, Modifiers::CTRL);
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
    fn a_message_quotes_a_bounded_head_of_what_was_typed() {
        let token: String = std::iter::repeat_n("abcdefghij", 15).collect();
        let err = parse(&format!("Ctrl+{token}")).unwrap_err();
        assert!(matches!(&err, ParseError::UnknownToken(t) if *t == token));
        assert_eq!(
            err.to_string(),
            format!("unknown hotkey token: \"{}…\"", &token[..ECHO_LIMIT])
        );
        assert_eq!(echoed("Numpad5"), "Numpad5");
        assert_eq!(echoed(&"x".repeat(ECHO_LIMIT)), "x".repeat(ECHO_LIMIT));
        assert_eq!(
            echoed(&"ж".repeat(ECHO_LIMIT + 1)).chars().count(),
            ECHO_LIMIT + 1
        );
    }

    #[test]
    fn empty_segment_errors() {
        assert!(matches!(parse("Ctrl++A"), Err(ParseError::UnknownToken(_))));
    }

    #[test]
    fn format_roundtrips_ctrl_shift_space() {
        let hk = parse("Ctrl+Shift+Space").unwrap();
        assert_eq!(format(&hk), "Ctrl+Shift+Space");
    }

    #[test]
    fn format_orders_modifiers_canonically() {
        let hk = parse("Shift+Ctrl+A").unwrap();
        assert_eq!(format(&hk), "Ctrl+Shift+A");
    }

    #[test]
    fn format_keeps_f_keys_uppercase() {
        let hk = parse("ctrl+f5").unwrap();
        assert_eq!(format(&hk), "Ctrl+F5");
    }

    #[test]
    fn modifier_symbols_parse_without_separators() {
        let hk = parse("⌥⌘J").unwrap();
        assert_eq!(hk.modifiers, Modifiers::ALT | Modifiers::WIN);
        assert_eq!(hk.key.as_str(), "J");
    }

    #[test]
    fn a_hotkey_is_one_hotkey_however_it_is_written() {
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
        assert_eq!(parse("⌥⌘").unwrap_err(), ParseError::MissingKey);
        assert_eq!(parse("⇧").unwrap_err(), ParseError::MissingKey);
    }

    #[test]
    fn an_unknown_key_after_symbols_is_still_reported_by_name() {
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
        assert_eq!(parse(&format(&hk)).unwrap(), hk);
        assert_eq!(parse("Win+J").unwrap(), hk);
    }

    #[test]
    fn a_key_that_types_something_needs_a_modifier() {
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
            assert_eq!(hk.modifiers, Modifiers::NONE);
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
    fn from_parts_agrees_with_parse_for_every_token() {
        for token in all_key_tokens() {
            let typed = parse(&format!("Ctrl+{token}")).unwrap_or_else(|e| {
                panic!("Ctrl+{token} must parse: {e}");
            });
            let built = from_parts(Modifiers::CTRL, &token)
                .unwrap_or_else(|e| panic!("from_parts(Ctrl, {token}) must build: {e}"));
            assert_eq!(built, typed, "token {token}");
        }
    }

    #[test]
    fn from_parts_applies_the_modifier_rule() {
        assert!(matches!(
            from_parts(Modifiers::NONE, "J"),
            Err(ParseError::MissingModifier(_))
        ));
        let f13 = from_parts(Modifiers::NONE, "F13").expect("the F-row may stand alone");
        assert_eq!(f13.modifiers, Modifiers::NONE);
        assert_eq!(f13.key.as_str(), "F13");
    }

    #[test]
    fn from_parts_canonicalises_its_token() {
        assert_eq!(
            from_parts(Modifiers::WIN, "esc").unwrap(),
            parse("Cmd+Escape").unwrap()
        );
        assert!(matches!(
            from_parts(Modifiers::CTRL, "Numpad5"),
            Err(ParseError::UnknownToken(t)) if t == "Numpad5"
        ));
    }

    #[test]
    fn all_key_tokens_is_the_whole_grammar_and_nothing_twice() {
        let tokens = all_key_tokens();
        let mut sorted = tokens.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), tokens.len(), "a token is listed twice");
        for token in &tokens {
            let hk = parse(&format!("Ctrl+{token}"))
                .unwrap_or_else(|e| panic!("Ctrl+{token} must parse: {e}"));
            assert_eq!(
                hk.key.as_str(),
                token,
                "the listed token must already be canonical"
            );
        }
        for spelling in ["Return", "PgUp", "Del", "Ins", "␣", "⎋", "↩", "←"] {
            let hk = parse(&format!("Ctrl+{spelling}")).unwrap();
            assert!(
                tokens.iter().any(|t| t == hk.key.as_str()),
                "{spelling} canonicalises to {}, which is not listed",
                hk.key.as_str()
            );
        }
    }

    #[test]
    fn default_hotkey_is_ctrl_shift_space() {
        let hk = default_hotkey();
        assert_eq!(format(&hk), "Ctrl+Shift+Space");
    }

    #[cfg(not(target_os = "macos"))]
    #[test]
    fn vocabulary_has_a_glyph_for_every_character_in_egui_default_proportional_font() {
        let ctx = eframe::egui::Context::default();
        let _ = ctx.run_ui(eframe::egui::RawInput::default(), |_| {});
        let font = eframe::egui::FontId::proportional(14.0);
        let missing: String = ctx.fonts_mut(|fonts| {
            VOCABULARY
                .chars()
                .filter(|&c| !fonts.has_glyph(&font, c))
                .collect()
        });
        assert!(
            missing.is_empty(),
            "egui's default proportional font has no glyph for {missing:?} in VOCABULARY"
        );
    }
}
