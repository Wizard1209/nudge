/**
 * Capturing a hotkey from the keyboard — `docs/TASK-hotkey-native.md`.
 *
 * Two states, no clock. Everything that decides lives in `recorderStep`, a
 * pure function of state and input; it touches no window, no platform and no
 * event loop. Whether a key is bindable is decided outside it, by
 * `classifyPress`, and handed in already answered.
 */
import {
  formatHotkeyNative,
  hasModifier,
  isFunctionKey,
  keyCodeName,
  tokenForKeyCode,
  type Hotkey,
} from "./hotkey";
import { detectPlatform, type Platform } from "./platform";

export type RecorderState = "idle" | "armed";

/** Why a pressed key cannot be bound. Both are the table's one "not bindable"
 * row; the reason only shapes the hint. */
export type NotBindableReason =
  /** Outside the §5 vocabulary — the keypad, a lock key, the media row. */
  | "outsideGrammar"
  /** In the grammar, but pressed bare, and only F1-F24 may stand alone (§5). */
  | "needsModifier";

/** A press edge, already judged. */
export type PressVerdict =
  | { kind: "bindable"; hotkey: Hotkey }
  | { kind: "bareEscape" }
  | { kind: "notBindable"; keyName: string; reason: NotBindableReason };

export type RecorderInput =
  | { kind: "record" }
  | { kind: "press"; verdict: PressVerdict }
  | { kind: "cancel" }
  | { kind: "focusLost" };

export type RecorderEffect =
  /** Nothing happens — every "—" cell of the table. */
  | { kind: "none" }
  /** Field := the combination, canonically spelled. */
  | { kind: "captured"; hotkey: Hotkey }
  /**
   * Field := the saved hotkey. The recorder remembers nothing when it arms:
   * every cancelling path re-reads `config.json`, so a value changed on disk
   * while the field was armed is the one that comes back.
   */
  | { kind: "restoreSaved" }
  /** Stays armed, with a hint naming the key. */
  | { kind: "hint"; keyName: string; reason: NotBindableReason };

export interface Transition {
  state: RecorderState;
  effect: RecorderEffect;
}

const STAY_IDLE: Transition = { state: "idle", effect: { kind: "none" } };

/**
 * The decision table, one cell at a time. No guards and no conditions outside
 * it: a chord of modifiers alone never arrives here (it is not a press edge),
 * an unbindable key does not disarm — disarming would leave a mistyped chord
 * looking like a recorded one — and bare Esc cancels because a hotkey needs a
 * modifier, so a lone Esc can never be one.
 */
export function recorderStep(
  state: RecorderState,
  input: RecorderInput,
): Transition {
  if (state === "idle") {
    // One row of the table has a cell here; the other five are "—".
    if (input.kind === "record") {
      return { state: "armed", effect: { kind: "none" } };
    }
    return STAY_IDLE;
  }

  switch (input.kind) {
    case "record":
      // "—": already armed, and arming twice is not a second recording.
      return { state: "armed", effect: { kind: "none" } };
    case "press":
      return armedPress(input.verdict);
    case "cancel":
      return { state: "idle", effect: { kind: "restoreSaved" } };
    case "focusLost":
      return { state: "idle", effect: { kind: "restoreSaved" } };
  }
}

/** The three Press cells of the Armed column. */
function armedPress(verdict: PressVerdict): Transition {
  switch (verdict.kind) {
    case "bindable":
      return { state: "idle", effect: { kind: "captured", hotkey: verdict.hotkey } };
    case "bareEscape":
      return { state: "idle", effect: { kind: "restoreSaved" } };
    case "notBindable":
      return {
        state: "armed",
        effect: {
          kind: "hint",
          keyName: verdict.keyName,
          reason: verdict.reason,
        },
      };
  }
}

/** The parts of a KeyboardEvent the recorder reads. `code` and nothing else
 * for the key: a hotkey follows the physical cap, not the character it types. */
export interface PressEdge {
  code: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
  /** True on an auto-repeat, which is not a press edge. */
  repeat?: boolean;
}

/**
 * Codes that carry no key of their own. A chord of modifiers is not an event:
 * holding ⌥⌘ neither accepts nor refuses.
 *
 * `fn` is here rather than among the refusals because it is not a key a user
 * chooses — macOS sets it by itself for the arrows and the F-row, and it
 * reaches neither the registration call nor the app (§5).
 */
const MODIFIER_CODES = new Set([
  "ControlLeft",
  "ControlRight",
  "AltLeft",
  "AltRight",
  "ShiftLeft",
  "ShiftRight",
  "MetaLeft",
  "MetaRight",
  "OSLeft",
  "OSRight",
  "AltGraph",
  "Fn",
  "FnLock",
]);

/**
 * Judge one press edge. Returns null when the event is not an input to the
 * recorder at all — a modifier going down, or an auto-repeat.
 *
 * This is the "decided outside" half of the table: it knows the platform's
 * key codes and the §5 grammar; `recorderStep` knows neither.
 */
export function classifyPress(e: PressEdge): PressVerdict | null {
  if (e.repeat) return null;
  if (MODIFIER_CODES.has(e.code)) return null;

  const mods = {
    ctrl: e.ctrlKey,
    alt: e.altKey,
    shift: e.shiftKey,
    win: e.metaKey,
  };

  if (e.code === "Escape" && !hasModifier(mods)) return { kind: "bareEscape" };

  const token = tokenForKeyCode(e.code);
  if (token === null) {
    return {
      kind: "notBindable",
      keyName: keyCodeName(e.code),
      reason: "outsideGrammar",
    };
  }
  if (!hasModifier(mods) && !isFunctionKey(token)) {
    return {
      kind: "notBindable",
      keyName: keyCodeName(e.code),
      reason: "needsModifier",
    };
  }
  return { kind: "bindable", hotkey: { ...mods, key: token } };
}

/** The hint shown while the recorder stays armed, naming the key that was
 * refused — the user has to see which press was thrown away. */
export function hintText(
  keyName: string,
  reason: NotBindableReason,
  platform: Platform = detectPlatform(),
): string {
  const mods = platform === "macos" ? "⌃ ⌥ ⇧ ⌘" : "Ctrl, Alt, Shift, Win";
  const fRow = platform === "macos" ? "F1–F20" : "F1–F24";
  return reason === "needsModifier"
    ? `${keyName} needs a modifier — hold ${mods} (only ${fRow} stand alone)`
    : `${keyName} cannot be a hotkey — press another key`;
}

/** Re-exported so a shell rendering a captured combination spells it the one
 * canonical way, rather than growing its own formatter. */
export { formatHotkeyNative };
