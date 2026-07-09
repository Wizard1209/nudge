export interface Hotkey {
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  win: boolean;
  /** Canonical key token: A-Z, 0-9, F1-F24, SPACE, ENTER, TAB, ESCAPE, BACKSPACE. */
  key: string;
}

export type ParseResult =
  | { ok: true; hotkey: Hotkey }
  | { ok: false; error: ParseError };

export type ParseError =
  | { kind: "empty" }
  | { kind: "missingKey" }
  | { kind: "multipleKeys" }
  | { kind: "unknownToken"; token: string };

const MODIFIER_ALIASES: Record<string, "ctrl" | "alt" | "shift" | "win"> = {
  CTRL: "ctrl",
  CONTROL: "ctrl",
  ALT: "alt",
  SHIFT: "shift",
  WIN: "win",
  SUPER: "win",
  META: "win",
  CMD: "win",
};

const NAMED_KEY_ALIASES: Record<string, string> = {
  SPACE: "SPACE",
  ENTER: "ENTER",
  RETURN: "ENTER",
  TAB: "TAB",
  ESC: "ESCAPE",
  ESCAPE: "ESCAPE",
  BACKSPACE: "BACKSPACE",
};

/** Canonicalize a non-modifier token into its key form, or null if unsupported. */
function canonicalizeKey(upper: string): string | null {
  if (/^[A-Z0-9]$/.test(upper)) return upper;
  const fMatch = /^F([1-9]|1[0-9]|2[0-4])$/.exec(upper);
  if (fMatch) return upper;
  return NAMED_KEY_ALIASES[upper] ?? null;
}

export function parseHotkey(input: string): ParseResult {
  if (input.trim() === "") {
    return { ok: false, error: { kind: "empty" } };
  }

  const tokens = input.split("+").map((t) => t.trim());

  const hotkey: Hotkey = {
    ctrl: false,
    alt: false,
    shift: false,
    win: false,
    key: "",
  };
  let keyCount = 0;

  for (const token of tokens) {
    const upper = token.toUpperCase();
    const modifier = MODIFIER_ALIASES[upper];
    if (modifier) {
      hotkey[modifier] = true;
      continue;
    }
    const key = canonicalizeKey(upper);
    if (key === null) {
      return { ok: false, error: { kind: "unknownToken", token } };
    }
    hotkey.key = key;
    keyCount++;
  }

  if (keyCount === 0) return { ok: false, error: { kind: "missingKey" } };
  if (keyCount > 1) return { ok: false, error: { kind: "multipleKeys" } };

  return { ok: true, hotkey };
}

export interface Modifiers {
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  win: boolean;
}

export type CaptureOutcome =
  | { kind: "captured"; hotkey: Hotkey }
  | { kind: "unsupported" }
  | { kind: "cancel" }
  | { kind: "waiting" };

/** Normalize a raw KeyboardEvent.key string into a canonicalizeKey token. */
function keyToToken(raw: string): string {
  if (raw === " ") return "SPACE";
  return raw.toUpperCase();
}

/**
 * Per-keydown recorder decision. `keys` is the set of non-modifier keys
 * currently down (raw `KeyboardEvent.key` strings). Mirrors the reference
 * `decide_capture`: bare Escape cancels, but Escape with any modifier is a
 * real combo; the first supported key is captured; an unsupported key keeps
 * recording with a hint. Pure — the Svelte recorder and unit tests share it.
 */
export function decideCapture(mods: Modifiers, keys: string[]): CaptureOutcome {
  const first = keys[0];
  if (first === undefined) return { kind: "waiting" };

  const token = keyToToken(first);
  const noModifiers = !(mods.ctrl || mods.alt || mods.shift || mods.win);
  if (token === "ESCAPE" && noModifiers) return { kind: "cancel" };

  const key = canonicalizeKey(token);
  if (key === null) return { kind: "unsupported" };

  return {
    kind: "captured",
    hotkey: {
      ctrl: mods.ctrl,
      alt: mods.alt,
      shift: mods.shift,
      win: mods.win,
      key,
    },
  };
}

const NAMED_KEY_LABELS: Record<string, string> = {
  SPACE: "Space",
  ENTER: "Enter",
  TAB: "Tab",
  ESCAPE: "Escape",
  BACKSPACE: "Backspace",
};

/** Named-key token → Electron accelerator token. Letters/digits/F-keys pass through. */
const ACCELERATOR_KEYS: Record<string, string> = {
  SPACE: "Space",
  ENTER: "Return",
  TAB: "Tab",
  ESCAPE: "Escape",
  BACKSPACE: "Backspace",
};

/**
 * Convert a parsed Hotkey into an Electron accelerator string for
 * `globalShortcut.register`. Modifiers map to Control/Alt/Shift/Super; the key
 * maps via ACCELERATOR_KEYS (named keys) or passes through (A-Z/0-9/F1-F24).
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

/** Render a hotkey as its canonical label (the inverse of parseHotkey). */
export function formatHotkey(hk: Hotkey): string {
  const parts: string[] = [];
  if (hk.ctrl) parts.push("Ctrl");
  if (hk.alt) parts.push("Alt");
  if (hk.shift) parts.push("Shift");
  if (hk.win) parts.push("Win");
  parts.push(NAMED_KEY_LABELS[hk.key] ?? hk.key);
  return parts.join("+");
}
