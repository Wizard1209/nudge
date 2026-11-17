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

export type NotBindableReason =
  | "outsideGrammar"
  | "needsModifier";

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
  | { kind: "none" }
  | { kind: "captured"; hotkey: Hotkey }
  | { kind: "restoreSaved" }
  | { kind: "hint"; keyName: string; reason: NotBindableReason };

export interface Transition {
  state: RecorderState;
  effect: RecorderEffect;
}

const STAY_IDLE: Transition = { state: "idle", effect: { kind: "none" } };

export function recorderStep(
  state: RecorderState,
  input: RecorderInput,
): Transition {
  if (state === "idle") {
    if (input.kind === "record") {
      return { state: "armed", effect: { kind: "none" } };
    }
    return STAY_IDLE;
  }

  switch (input.kind) {
    case "record":
      return { state: "armed", effect: { kind: "none" } };
    case "press":
      return armedPress(input.verdict);
    case "cancel":
      return { state: "idle", effect: { kind: "restoreSaved" } };
    case "focusLost":
      return { state: "idle", effect: { kind: "restoreSaved" } };
  }
}

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

export interface PressEdge {
  code: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
  repeat?: boolean;
}

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

export { formatHotkeyNative };
