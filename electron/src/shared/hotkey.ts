/**
 * The §5 hotkey grammar: parsing a user-readable string into modifiers + one
 * key, writing it back in this platform's canonical spelling, and naming the
 * physical key an event carries.
 *
 * This mirrors the Rust `src/hotkey.rs` token for token — the spec is the
 * contract both implementations follow, so the tables here are the same
 * tables, in the same order.
 */
import { detectPlatform, type Platform } from "./platform";

export interface Hotkey {
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  win: boolean;
  /**
   * Canonical key token: A-Z, 0-9, F1-F24, one of the named keys
   * (SPACE/ENTER/TAB/ESCAPE/BACKSPACE, the arrows, the navigation cluster),
   * or a symbol cap (a single character out of SYMBOL_KEYS).
   */
  key: string;
}

export type ParseResult =
  | { ok: true; hotkey: Hotkey }
  | { ok: false; error: ParseError };

export type ParseError =
  | { kind: "empty" }
  | { kind: "missingKey" }
  | { kind: "multipleKeys" }
  | { kind: "unknownToken"; token: string }
  /** "J" — a key that types something, with nothing held. §5. */
  | { kind: "missingModifier"; key: string };

/**
 * The macOS symbol for each modifier, in the order Apple prints them in a
 * menu: Control, Option, Shift, Command.
 */
const MOD_GLYPHS: ReadonlyArray<[keyof Modifiers, string]> = [
  ["ctrl", "⌃"],
  ["alt", "⌥"],
  ["shift", "⇧"],
  ["win", "⌘"],
];

/**
 * The modifier a leading glyph stands for, or null if the character is not a
 * modifier glyph.
 *
 * Accepted on every platform, not just macOS: a config carried from a Mac to
 * a PC keeps its meaning, and the settings window on macOS writes this form
 * into the file.
 */
function modifierGlyph(c: string): keyof Modifiers | null {
  if (c === "⊞") return "win";
  const row = MOD_GLYPHS.find(([, glyph]) => glyph === c);
  return row ? row[0] : null;
}

const MODIFIER_WORDS: Record<string, keyof Modifiers> = {
  CTRL: "ctrl",
  CONTROL: "ctrl",
  ALT: "alt",
  OPTION: "alt",
  OPT: "alt",
  SHIFT: "shift",
  WIN: "win",
  SUPER: "win",
  META: "win",
  CMD: "win",
  COMMAND: "win",
};

/**
 * The symbol caps, named by their unshifted legend (§5 rule 6). One character
 * each, so they are their own canonical token and their own label on both
 * platforms. The shifted twin of a cap (`?` for `/`, `+` for `=`) is not a
 * spelling: it would be a second name for one binding, and `+` is the
 * separator besides.
 */
const SYMBOL_KEYS = "-=[]\\;',./`";

interface NamedKey {
  /** Canonical token. */
  token: string;
  /** Every accepted spelling, uppercased. */
  spellings: string[];
  /** What `formatHotkey` emits. */
  label: string;
  /** What `formatHotkeyNative` shows on macOS; empty where the cap carries a
   * word (`Space`) or has no cap there at all (`Insert`). */
  glyph: string;
}

/**
 * Every key that is not a letter, a digit or F1-F24 — one row per physical
 * key, carrying the spellings §5 lists (including the Mac symbols, so a
 * shortcut written the way a Mac writes it round-trips).
 */
const NAMED_KEYS: NamedKey[] = [
  { token: "SPACE", spellings: ["SPACE", "␣"], label: "Space", glyph: "" },
  { token: "ENTER", spellings: ["ENTER", "RETURN", "↩", "⏎", "⌤"], label: "Enter", glyph: "↩" },
  { token: "TAB", spellings: ["TAB", "⇥"], label: "Tab", glyph: "⇥" },
  { token: "ESCAPE", spellings: ["ESCAPE", "ESC", "⎋"], label: "Escape", glyph: "⎋" },
  { token: "BACKSPACE", spellings: ["BACKSPACE", "⌫"], label: "Backspace", glyph: "⌫" },
  { token: "LEFT", spellings: ["LEFT", "←"], label: "Left", glyph: "←" },
  { token: "RIGHT", spellings: ["RIGHT", "→"], label: "Right", glyph: "→" },
  { token: "UP", spellings: ["UP", "↑"], label: "Up", glyph: "↑" },
  { token: "DOWN", spellings: ["DOWN", "↓"], label: "Down", glyph: "↓" },
  { token: "HOME", spellings: ["HOME", "↖"], label: "Home", glyph: "↖" },
  { token: "END", spellings: ["END", "↘"], label: "End", glyph: "↘" },
  { token: "PAGEUP", spellings: ["PAGEUP", "PGUP", "⇞"], label: "PageUp", glyph: "⇞" },
  { token: "PAGEDOWN", spellings: ["PAGEDOWN", "PGDN", "⇟"], label: "PageDown", glyph: "⇟" },
  { token: "INSERT", spellings: ["INSERT", "INS"], label: "Insert", glyph: "" },
  { token: "DELETE", spellings: ["DELETE", "DEL", "⌦"], label: "Delete", glyph: "⌦" },
];

/**
 * The canonical token of an F-row spelling, or null if it is not one. The
 * digits are read as a number, not as text, so `F07` is the same key as `F7`
 * and canonicalizes to it — the same as the Rust side, where the digits go
 * through `parse::<u8>()`.
 */
function functionKeyToken(upper: string): string | null {
  const m = /^F([0-9]+)$/.exec(upper);
  if (!m) return null;
  const n = Number(m[1]);
  // The upper bound is the byte the Rust parse reads into: past it there is no
  // number, so `F999` is an unknown token rather than an out-of-range F-key.
  if (n > 255) return null;
  return n >= 1 && n <= 24 ? `F${n}` : null;
}

/** Whether a canonical key token is one of F1-F24 — the keys that type
 * nothing, and so the only ones allowed to stand alone (§5). */
export function isFunctionKey(key: string): boolean {
  return functionKeyToken(key) !== null;
}

/**
 * The canonical form of an already-uppercased key token, or null if it is not
 * a key §5 knows. Kept separate from `parseHotkey` so the recorder can name a
 * key without re-parsing a whole string.
 */
function canonicalizeKey(upper: string): string | null {
  if ([...upper].length === 1 && (/^[A-Z0-9]$/.test(upper) || SYMBOL_KEYS.includes(upper))) {
    return upper;
  }
  const fKey = functionKeyToken(upper);
  if (fKey !== null) return fKey;
  return NAMED_KEYS.find((k) => k.spellings.includes(upper))?.token ?? null;
}

type Modifiers = Pick<Hotkey, "ctrl" | "alt" | "shift" | "win">;

/**
 * Parse a user-readable hotkey string ("Ctrl+Shift+Space", "⌥⌘J", "⇧/").
 *
 * - Case-insensitive; whitespace around `+` and the whole string is ignored.
 * - Modifier glyphs need no `+` between them, because that is how macOS
 *   writes a shortcut and the settings field holds exactly that string.
 * - Exactly one non-modifier token is required.
 * - Duplicate modifiers ("Ctrl+Ctrl+A") collapse silently (§5 rule 5).
 */
export function parseHotkey(input: string): ParseResult {
  const trimmed = input.trim();
  if (trimmed === "") return { ok: false, error: { kind: "empty" } };

  const hotkey: Hotkey = {
    ctrl: false,
    alt: false,
    shift: false,
    win: false,
    key: "",
  };
  let keyCount = 0;

  for (const segment of trimmed.split("+")) {
    let tok = segment.trim();
    if (tok === "") {
      // "Ctrl++A" or a trailing "+" — a malformed empty token. `+` is only
      // the separator (§5 rule 6), so this is never the `=` cap.
      return { ok: false, error: { kind: "unknownToken", token: "" } };
    }
    // Peel leading modifier glyphs off the segment. This is what makes the
    // separator optional: "⌥⌘J" arrives as one segment and leaves "J".
    let head = [...tok][0];
    let mod = modifierGlyph(head);
    while (mod !== null) {
      hotkey[mod] = true;
      tok = tok.slice(head.length).trimStart();
      head = [...tok][0] ?? "";
      mod = head === "" ? null : modifierGlyph(head);
    }
    if (tok === "") continue; // the segment was glyphs only — legal.

    const upper = tok.toUpperCase();
    const word = MODIFIER_WORDS[upper];
    if (word) {
      hotkey[word] = true;
      continue;
    }
    const key = canonicalizeKey(upper);
    if (key === null) {
      return { ok: false, error: { kind: "unknownToken", token: tok } };
    }
    if (keyCount > 0) return { ok: false, error: { kind: "multipleKeys" } };
    hotkey.key = key;
    keyCount++;
  }

  if (keyCount === 0) return { ok: false, error: { kind: "missingKey" } };
  // §5: at least one modifier, except for the F-row — the only keys that type
  // nothing. Registration takes a bare key and then owns it machine-wide.
  if (!hasModifier(hotkey) && !isFunctionKey(hotkey.key)) {
    return { ok: false, error: { kind: "missingModifier", key: hotkey.key } };
  }
  return { ok: true, hotkey };
}

/** A parse error in one sentence — the Display impl of the Rust `ParseError`,
 * word for word, because it is the same message in the same banner. */
export function describeParseError(error: ParseError): string {
  switch (error.kind) {
    case "empty":
      return "hotkey string is empty";
    case "missingKey":
      return 'hotkey has only modifiers, no key (e.g. need "Ctrl+Shift+A", not "Ctrl+Shift")';
    case "multipleKeys":
      return "hotkey has more than one non-modifier key";
    case "unknownToken":
      return `unknown hotkey token: "${error.token}"`;
    case "missingModifier":
      return (
        `hotkey "${error.key}" has no modifier — that would take the key away ` +
        "from every other application (only F1-F24 may stand alone)"
      );
  }
}

export function hasModifier(mods: Modifiers): boolean {
  return mods.ctrl || mods.alt || mods.shift || mods.win;
}

/** The label `formatHotkey` shows for a canonical token. */
function keyLabel(token: string): string {
  return NAMED_KEYS.find((k) => k.token === token)?.label ?? token;
}

/**
 * The word form of a hotkey: "Ctrl+Shift+Space". Modifiers are always emitted
 * in the order Control, Option/Alt, Shift, Command/Win — the order Apple
 * prints them in a menu. The fourth modifier is one modifier; only its name
 * differs, so a Mac reads `Cmd` where a PC reads `Win`.
 */
export function formatHotkey(hk: Hotkey, platform: Platform = detectPlatform()): string {
  const parts: string[] = [];
  if (hk.ctrl) parts.push("Ctrl");
  if (hk.alt) parts.push("Alt");
  if (hk.shift) parts.push("Shift");
  if (hk.win) parts.push(platform === "macos" ? "Cmd" : "Win");
  parts.push(keyLabel(hk.key));
  return parts.join("+");
}

/**
 * The spelling to *show and store* on this platform — on macOS the symbol
 * form, `⌥⌘J`, which is what every other Mac app prints and what the keycaps
 * say; elsewhere the word form unchanged.
 *
 * A rendering, not a second format: `parseHotkey` accepts what this emits, so
 * the settings field can hold it and Save can put it in the config unchanged.
 *
 * The key half is symbolic only where a Mac keycap is: `⌘Space`, not `⌘␣`,
 * because the space bar has no legend and Apple's own menus write the word.
 */
export function formatHotkeyNative(
  hk: Hotkey,
  platform: Platform = detectPlatform(),
): string {
  if (platform !== "macos") return formatHotkey(hk, platform);
  let out = "";
  for (const [mod, glyph] of MOD_GLYPHS) {
    if (hk[mod]) out += glyph;
  }
  const named = NAMED_KEYS.find((k) => k.token === hk.key);
  out += named && named.glyph !== "" ? named.glyph : keyLabel(hk.key);
  return out;
}

/**
 * The whole accepted vocabulary in one sentence, spelled the way this platform
 * spells it. Printed under the settings field, because that is where a hotkey
 * is set and so the list is standing information, not an error message (§9).
 */
export function vocabulary(platform: Platform = detectPlatform()): string {
  const head =
    platform === "macos"
      ? "Modifiers ⌃ ⌥ ⇧ ⌘ (or Ctrl, Option, Shift, Cmd) plus one key: A–Z, 0–9, F1–F20,"
      : "Modifiers Ctrl, Alt, Shift, Win plus one key: A–Z, 0–9, F1–F24,";
  return (
    `${head} Space, Enter, Tab, Esc, Backspace, ← ↑ → ↓, ` +
    "Home, End, PageUp, PageDown, Insert, Delete, " +
    "or a symbol cap: - = [ ] \\ ; ' , . / `"
  );
}

/**
 * KeyboardEvent.code → §5 token. The physical key, never `KeyboardEvent.key`:
 * a hotkey is a position on the keyboard (§5 rule 7), so the cap that types
 * `о` on a Russian layout still records as `J`.
 *
 * This is the prototype's stand-in for the platform key code the task doc
 * asks the Rust app to read straight from the OS event. Electron gives the
 * renderer a W3C code, which is already layout-independent, so no translation
 * table stands between the event and the token.
 */
const KEY_CODE_TOKENS: Record<string, string> = {
  Space: "SPACE",
  Enter: "ENTER",
  Tab: "TAB",
  Escape: "ESCAPE",
  Backspace: "BACKSPACE",
  ArrowLeft: "LEFT",
  ArrowRight: "RIGHT",
  ArrowUp: "UP",
  ArrowDown: "DOWN",
  Home: "HOME",
  End: "END",
  PageUp: "PAGEUP",
  PageDown: "PAGEDOWN",
  Insert: "INSERT",
  Delete: "DELETE",
  Minus: "-",
  Equal: "=",
  BracketLeft: "[",
  BracketRight: "]",
  Backslash: "\\",
  Semicolon: ";",
  Quote: "'",
  Comma: ",",
  Period: ".",
  Slash: "/",
  Backquote: "`",
};

/** The §5 token a physical key carries, or null if that key is outside the
 * grammar (the keypad, the lock keys, the media row). */
export function tokenForKeyCode(code: string): string | null {
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter) return letter[1];
  const digit = /^Digit([0-9])$/.exec(code);
  if (digit) return digit[1];
  const fKey = functionKeyToken(code);
  if (fKey !== null) return fKey;
  return KEY_CODE_TOKENS[code] ?? null;
}

/** The keypad, spelled so it can never be read as the digit row (§5: the
 * keypad is outside the grammar, and a refusal has to say which key it means). */
const KEYPAD_NAMES: Record<string, string> = {
  NumpadDecimal: "Keypad .",
  NumpadAdd: "Keypad +",
  NumpadSubtract: "Keypad -",
  NumpadMultiply: "Keypad *",
  NumpadDivide: "Keypad /",
  NumpadEnter: "Keypad Enter",
  NumpadEqual: "Keypad =",
  NumpadComma: "Keypad ,",
  NumpadClear: "Keypad Clear",
  NumLock: "Keypad Clear",
};

/** Codes with no §5 token but a name a user recognises on the keycap. */
const OTHER_KEY_NAMES: Record<string, string> = {
  CapsLock: "Caps Lock",
  ContextMenu: "Menu",
  PrintScreen: "Print Screen",
  ScrollLock: "Scroll Lock",
  Pause: "Pause",
  IntlBackslash: "Section",
  IntlRo: "Ro",
  IntlYen: "Yen",
  Help: "Help",
  Power: "Power",
  Eject: "Eject",
};

/**
 * What to call a physical key in a refusal. A key inside the grammar is named
 * by its §5 label; the keypad by "Keypad N"; anything else by its keycap name,
 * falling back to the raw code so the message still says *which* key.
 */
export function keyCodeName(code: string): string {
  const token = tokenForKeyCode(code);
  if (token !== null) return keyLabel(token);
  const numpadDigit = /^Numpad([0-9])$/.exec(code);
  if (numpadDigit) return `Keypad ${numpadDigit[1]}`;
  return KEYPAD_NAMES[code] ?? OTHER_KEY_NAMES[code] ?? (code || "Unknown key");
}

/** Named-key token → Electron accelerator token. Letters, digits, F-keys and
 * the symbol caps pass through unchanged. */
const ACCELERATOR_KEYS: Record<string, string> = {
  SPACE: "Space",
  ENTER: "Return",
  TAB: "Tab",
  ESCAPE: "Escape",
  BACKSPACE: "Backspace",
  LEFT: "Left",
  RIGHT: "Right",
  UP: "Up",
  DOWN: "Down",
  HOME: "Home",
  END: "End",
  PAGEUP: "PageUp",
  PAGEDOWN: "PageDown",
  INSERT: "Insert",
  DELETE: "Delete",
};

/**
 * A parsed Hotkey as an Electron accelerator for `globalShortcut.register`.
 *
 * This is the one place the prototype cannot honor the task doc's "one
 * identifier from the key going down to the registration call": Electron takes
 * a *string* and does its own lookup, so the token↔accelerator table below is
 * a second table that has to agree with the one above by hand. The Rust app
 * hands the OS the same key code the event carried and has no such table.
 */
export function hotkeyToAccelerator(hk: Hotkey): string {
  const parts: string[] = [];
  if (hk.ctrl) parts.push("Control");
  if (hk.alt) parts.push("Alt");
  if (hk.shift) parts.push("Shift");
  if (hk.win) parts.push("Super");
  parts.push(ACCELERATOR_KEYS[hk.key] ?? hk.key);
  return parts.join("+");
}
