import { detectPlatform, type Platform } from "./platform";

export interface Hotkey {
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  win: boolean;
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
  | { kind: "missingModifier"; key: string };

const MOD_GLYPHS: ReadonlyArray<[keyof Modifiers, string]> = [
  ["ctrl", "⌃"],
  ["alt", "⌥"],
  ["shift", "⇧"],
  ["win", "⌘"],
];

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

const SYMBOL_KEYS = "-=[]\\;',./`";

interface NamedKey {
  token: string;
  spellings: string[];
  label: string;
  glyph: string;
}

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

function functionKeyToken(upper: string): string | null {
  const m = /^F([0-9]+)$/.exec(upper);
  if (!m) return null;
  const n = Number(m[1]);
  if (n > 255) return null;
  return n >= 1 && n <= 24 ? `F${n}` : null;
}

export function isFunctionKey(key: string): boolean {
  return functionKeyToken(key) !== null;
}

function canonicalizeKey(upper: string): string | null {
  if ([...upper].length === 1 && (/^[A-Z0-9]$/.test(upper) || SYMBOL_KEYS.includes(upper))) {
    return upper;
  }
  const fKey = functionKeyToken(upper);
  if (fKey !== null) return fKey;
  return NAMED_KEYS.find((k) => k.spellings.includes(upper))?.token ?? null;
}

type Modifiers = Pick<Hotkey, "ctrl" | "alt" | "shift" | "win">;

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
      return { ok: false, error: { kind: "unknownToken", token: "" } };
    }
    let head = [...tok][0];
    let mod = modifierGlyph(head);
    while (mod !== null) {
      hotkey[mod] = true;
      tok = tok.slice(head.length).trimStart();
      head = [...tok][0] ?? "";
      mod = head === "" ? null : modifierGlyph(head);
    }
    if (tok === "") continue;

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
  if (!hasModifier(hotkey) && !isFunctionKey(hotkey.key)) {
    return { ok: false, error: { kind: "missingModifier", key: hotkey.key } };
  }
  return { ok: true, hotkey };
}

export const ECHO_LIMIT = 40;

export function echoed(text: string): string {
  const chars = [...text];
  return chars.length > ECHO_LIMIT ? `${chars.slice(0, ECHO_LIMIT).join("")}…` : text;
}

export function describeParseError(error: ParseError): string {
  switch (error.kind) {
    case "empty":
      return "hotkey string is empty";
    case "missingKey":
      return 'hotkey has only modifiers, no key (e.g. need "Ctrl+Shift+A", not "Ctrl+Shift")';
    case "multipleKeys":
      return "hotkey has more than one non-modifier key";
    case "unknownToken":
      return `unknown hotkey token: "${echoed(error.token)}"`;
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

function keyLabel(token: string): string {
  return NAMED_KEYS.find((k) => k.token === token)?.label ?? token;
}

export function formatHotkey(hk: Hotkey, platform: Platform = detectPlatform()): string {
  const parts: string[] = [];
  if (hk.ctrl) parts.push("Ctrl");
  if (hk.alt) parts.push("Alt");
  if (hk.shift) parts.push("Shift");
  if (hk.win) parts.push(platform === "macos" ? "Cmd" : "Win");
  parts.push(keyLabel(hk.key));
  return parts.join("+");
}

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

export function tokenForKeyCode(code: string): string | null {
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter) return letter[1];
  const digit = /^Digit([0-9])$/.exec(code);
  if (digit) return digit[1];
  const fKey = functionKeyToken(code);
  if (fKey !== null) return fKey;
  return KEY_CODE_TOKENS[code] ?? null;
}

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

export function keyCodeName(code: string): string {
  const token = tokenForKeyCode(code);
  if (token !== null) return keyLabel(token);
  const numpadDigit = /^Numpad([0-9])$/.exec(code);
  if (numpadDigit) return `Keypad ${numpadDigit[1]}`;
  return KEYPAD_NAMES[code] ?? OTHER_KEY_NAMES[code] ?? (code || "Unknown key");
}

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

export function hotkeyToAccelerator(hk: Hotkey): string {
  const parts: string[] = [];
  if (hk.ctrl) parts.push("Control");
  if (hk.alt) parts.push("Alt");
  if (hk.shift) parts.push("Shift");
  if (hk.win) parts.push("Super");
  parts.push(ACCELERATOR_KEYS[hk.key] ?? hk.key);
  return parts.join("+");
}
