import { describe, it, expect } from "vitest";
import {
  classifyPress,
  hintText,
  recorderStep,
  type PressEdge,
  type PressVerdict,
  type RecorderInput,
  type RecorderState,
} from "../../src/shared/hotkeyRecorder";
import { parseHotkey, type Hotkey } from "../../src/shared/hotkey";

const hk = (label: string): Hotkey => {
  const r = parseHotkey(label);
  if (!r.ok) throw new Error(`bad fixture: ${label}`);
  return r.hotkey;
};

const bindable: PressVerdict = { kind: "bindable", hotkey: hk("Alt+Cmd+J") };
const bareEscape: PressVerdict = { kind: "bareEscape" };
const notBindable: PressVerdict = {
  kind: "notBindable",
  keyName: "Keypad 5",
  reason: "outsideGrammar",
};

const RECORD: RecorderInput = { kind: "record" };
const CANCEL: RecorderInput = { kind: "cancel" };
const FOCUS_LOST: RecorderInput = { kind: "focusLost" };
const press = (verdict: PressVerdict): RecorderInput => ({ kind: "press", verdict });

// docs/TASK-hotkey-native.md — the decision table, every cell, both columns.
// Nothing decides outside it, so nothing else may be asserted here.
describe("recorderStep — the decision table", () => {
  describe("Idle", () => {
    it("Record → Armed", () => {
      expect(recorderStep("idle", RECORD)).toEqual({
        state: "armed",
        effect: { kind: "none" },
      });
    });

    it("Press (bindable) → —", () => {
      expect(recorderStep("idle", press(bindable))).toEqual({
        state: "idle",
        effect: { kind: "none" },
      });
    });

    it("Press (bare Esc) → —", () => {
      expect(recorderStep("idle", press(bareEscape))).toEqual({
        state: "idle",
        effect: { kind: "none" },
      });
    });

    it("Press (not bindable) → —", () => {
      expect(recorderStep("idle", press(notBindable))).toEqual({
        state: "idle",
        effect: { kind: "none" },
      });
    });

    it("Cancel → —", () => {
      expect(recorderStep("idle", CANCEL)).toEqual({
        state: "idle",
        effect: { kind: "none" },
      });
    });

    it("Focus lost → —", () => {
      expect(recorderStep("idle", FOCUS_LOST)).toEqual({
        state: "idle",
        effect: { kind: "none" },
      });
    });
  });

  describe("Armed", () => {
    it("Record → — (arming twice is not a second recording)", () => {
      expect(recorderStep("armed", RECORD)).toEqual({
        state: "armed",
        effect: { kind: "none" },
      });
    });

    it("Press (bindable) → Idle, field := the combination", () => {
      expect(recorderStep("armed", press(bindable))).toEqual({
        state: "idle",
        effect: { kind: "captured", hotkey: hk("Alt+Cmd+J") },
      });
    });

    it("Press (bare Esc) → Idle, field := the saved hotkey", () => {
      expect(recorderStep("armed", press(bareEscape))).toEqual({
        state: "idle",
        effect: { kind: "restoreSaved" },
      });
    });

    it("Press (not bindable) → Armed, with a hint naming the key", () => {
      expect(recorderStep("armed", press(notBindable))).toEqual({
        state: "armed",
        effect: { kind: "hint", keyName: "Keypad 5", reason: "outsideGrammar" },
      });
    });

    it("Cancel → Idle, field := the saved hotkey", () => {
      expect(recorderStep("armed", CANCEL)).toEqual({
        state: "idle",
        effect: { kind: "restoreSaved" },
      });
    });

    it("Focus lost → Idle, field := the saved hotkey", () => {
      expect(recorderStep("armed", FOCUS_LOST)).toEqual({
        state: "idle",
        effect: { kind: "restoreSaved" },
      });
    });
  });

  it("covers all twelve cells and nothing else", () => {
    const states: RecorderState[] = ["idle", "armed"];
    const inputs: RecorderInput[] = [
      RECORD,
      press(bindable),
      press(bareEscape),
      press(notBindable),
      CANCEL,
      FOCUS_LOST,
    ];
    const cells = states.flatMap((s) =>
      inputs.map((i) => `${s}/${i.kind === "press" ? i.verdict.kind : i.kind}`),
    );
    expect(new Set(cells).size).toBe(12);
    // Every cell is total: no input leaves the recorder without a next state.
    for (const s of states) {
      for (const i of inputs) {
        expect(["idle", "armed"]).toContain(recorderStep(s, i).state);
      }
    }
  });

  it("never carries a remembered hotkey out of a cancelling path", () => {
    // The recorder has nothing to remember when it arms: the effect says
    // "re-read the saved value", it does not hand one back.
    for (const input of [press(bareEscape), CANCEL, FOCUS_LOST]) {
      const { effect } = recorderStep("armed", input);
      expect(effect).toEqual({ kind: "restoreSaved" });
    }
  });
});

const edge = (code: string, mods: Partial<PressEdge> = {}): PressEdge => ({
  code,
  ctrlKey: false,
  altKey: false,
  shiftKey: false,
  metaKey: false,
  ...mods,
});

describe("classifyPress — the bindability decided outside the table", () => {
  it("judges a chord by the physical cap, not the character it types", () => {
    // A Russian layout types `о` on this cap; `code` does not move.
    expect(classifyPress(edge("KeyJ", { altKey: true, metaKey: true }))).toEqual({
      kind: "bindable",
      hotkey: { ctrl: false, alt: true, shift: false, win: true, key: "J" },
    });
  });

  it("carries the modifiers held at that instant", () => {
    expect(
      classifyPress(edge("ArrowLeft", { altKey: true, metaKey: true })),
    ).toEqual({
      kind: "bindable",
      hotkey: { ctrl: false, alt: true, shift: false, win: true, key: "LEFT" },
    });
    expect(classifyPress(edge("Slash", { shiftKey: true }))).toEqual({
      kind: "bindable",
      hotkey: { ctrl: false, alt: false, shift: true, win: false, key: "/" },
    });
  });

  it("is not an event at all for a modifier going down", () => {
    for (const code of [
      "ControlLeft",
      "AltLeft",
      "ShiftRight",
      "MetaLeft",
      "Fn",
    ]) {
      expect(classifyPress(edge(code, { altKey: true, metaKey: true })), code).toBeNull();
    }
  });

  it("is not an event for an auto-repeat — only a press edge counts", () => {
    expect(
      classifyPress(edge("KeyJ", { altKey: true, metaKey: true, repeat: true })),
    ).toBeNull();
  });

  it("reads bare Esc as the cancel press, but Esc with a modifier as a combo", () => {
    expect(classifyPress(edge("Escape"))).toEqual({ kind: "bareEscape" });
    expect(classifyPress(edge("Escape", { ctrlKey: true }))).toEqual({
      kind: "bindable",
      hotkey: { ctrl: true, alt: false, shift: false, win: false, key: "ESCAPE" },
    });
  });

  it("refuses the keypad by name, never as the digit row", () => {
    expect(classifyPress(edge("Numpad5", { altKey: true }))).toEqual({
      kind: "notBindable",
      keyName: "Keypad 5",
      reason: "outsideGrammar",
    });
    expect(classifyPress(edge("Digit5", { altKey: true }))).toEqual({
      kind: "bindable",
      hotkey: { ctrl: false, alt: true, shift: false, win: false, key: "5" },
    });
  });

  it("refuses a bare letter by name, and takes a bare F13", () => {
    expect(classifyPress(edge("KeyJ"))).toEqual({
      kind: "notBindable",
      keyName: "J",
      reason: "needsModifier",
    });
    expect(classifyPress(edge("F13"))).toEqual({
      kind: "bindable",
      hotkey: { ctrl: false, alt: false, shift: false, win: false, key: "F13" },
    });
  });

  it("refuses a key outside the grammar by its keycap name", () => {
    expect(classifyPress(edge("CapsLock", { ctrlKey: true }))).toEqual({
      kind: "notBindable",
      keyName: "Caps Lock",
      reason: "outsideGrammar",
    });
  });
});

describe("hintText", () => {
  it("names the key and, when it is the reason, the missing modifier", () => {
    expect(hintText("Keypad 5", "outsideGrammar", "macos")).toContain("Keypad 5");
    expect(hintText("J", "needsModifier", "macos")).toContain("J");
    expect(hintText("J", "needsModifier", "macos")).toContain("⌘");
    expect(hintText("J", "needsModifier", "windows")).toContain("Win");
  });
});
