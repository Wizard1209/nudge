import { describe, it, expect } from "vitest";
import {
  parseHotkey,
  formatHotkey,
  decideCapture,
  hotkeyToAccelerator,
} from "../../src/shared/hotkey";

describe("parseHotkey", () => {
  it("parses a canonical combo into modifiers and key", () => {
    const result = parseHotkey("Ctrl+Shift+Space");
    expect(result).toEqual({
      ok: true,
      hotkey: { ctrl: true, alt: false, shift: true, win: false, key: "SPACE" },
    });
  });

  it("parses a single modifier and a letter key", () => {
    const result = parseHotkey("Alt+J");
    expect(result).toEqual({
      ok: true,
      hotkey: { ctrl: false, alt: true, shift: false, win: false, key: "J" },
    });
  });

  it("is case- and whitespace-insensitive", () => {
    expect(parseHotkey("  ctrl +  SHIFT +space  ")).toEqual(
      parseHotkey("Ctrl+Shift+Space"),
    );
  });

  it("accepts Win aliases (Super/Meta/Cmd)", () => {
    for (const alias of ["Win", "Super", "Meta", "Cmd"]) {
      expect(parseHotkey(`${alias}+J`)).toEqual({
        ok: true,
        hotkey: { ctrl: false, alt: false, shift: false, win: true, key: "J" },
      });
    }
  });

  it("canonicalizes named keys (Enter/Return -> ENTER, Esc -> ESCAPE)", () => {
    expect(parseHotkey("Ctrl+Enter").hotkey?.key).toBe("ENTER");
    expect(parseHotkey("Ctrl+Return").hotkey?.key).toBe("ENTER");
    expect(parseHotkey("Ctrl+Esc").hotkey?.key).toBe("ESCAPE");
    expect(parseHotkey("Ctrl+Escape").hotkey?.key).toBe("ESCAPE");
    expect(parseHotkey("Ctrl+Tab").hotkey?.key).toBe("TAB");
    expect(parseHotkey("Ctrl+Backspace").hotkey?.key).toBe("BACKSPACE");
  });

  it("accepts F1 through F24 but rejects F25", () => {
    for (let n = 1; n <= 24; n++) {
      expect(parseHotkey(`Ctrl+F${n}`).hotkey?.key).toBe(`F${n}`);
    }
    expect(parseHotkey("Ctrl+F25")).toEqual({
      ok: false,
      error: { kind: "unknownToken", token: "F25" },
    });
  });

  it("rejects an unsupported key token", () => {
    expect(parseHotkey("Ctrl+Home")).toEqual({
      ok: false,
      error: { kind: "unknownToken", token: "Home" },
    });
  });

  it("rejects an empty string", () => {
    expect(parseHotkey("")).toEqual({ ok: false, error: { kind: "empty" } });
    expect(parseHotkey("   ")).toEqual({ ok: false, error: { kind: "empty" } });
  });

  it("rejects modifiers with no key", () => {
    expect(parseHotkey("Ctrl+Shift")).toEqual({
      ok: false,
      error: { kind: "missingKey" },
    });
  });

  it("rejects more than one key", () => {
    expect(parseHotkey("Ctrl+A+B")).toEqual({
      ok: false,
      error: { kind: "multipleKeys" },
    });
  });

  it("rejects an empty segment (Ctrl++A)", () => {
    expect(parseHotkey("Ctrl++A")).toEqual({
      ok: false,
      error: { kind: "unknownToken", token: "" },
    });
  });
});

describe("decideCapture", () => {
  const noMods = { ctrl: false, alt: false, shift: false, win: false };

  it("keeps waiting when no non-modifier key is down", () => {
    expect(decideCapture({ ...noMods, ctrl: true }, [])).toEqual({
      kind: "waiting",
    });
  });

  it("cancels on bare Escape (no modifiers held)", () => {
    expect(decideCapture(noMods, ["Escape"])).toEqual({ kind: "cancel" });
  });

  it("captures Escape when a modifier is held (Ctrl+Esc is a real combo)", () => {
    expect(decideCapture({ ...noMods, ctrl: true }, ["Escape"])).toEqual({
      kind: "captured",
      hotkey: { ctrl: true, alt: false, shift: false, win: false, key: "ESCAPE" },
    });
  });

  it("captures a supported chord (Ctrl+Shift+A)", () => {
    expect(
      decideCapture({ ctrl: true, alt: false, shift: true, win: false }, ["a"]),
    ).toEqual({
      kind: "captured",
      hotkey: { ctrl: true, alt: false, shift: true, win: false, key: "A" },
    });
  });

  it("normalizes the space key", () => {
    expect(decideCapture({ ...noMods, ctrl: true }, [" "])).toEqual({
      kind: "captured",
      hotkey: { ctrl: true, alt: false, shift: false, win: false, key: "SPACE" },
    });
  });

  it("reports unsupported for a key outside the allowlist", () => {
    expect(decideCapture({ ...noMods, ctrl: true }, ["Home"])).toEqual({
      kind: "unsupported",
    });
  });

  it("decides on the first key when several are down", () => {
    expect(decideCapture({ ...noMods, alt: true }, ["b", "c"])).toEqual({
      kind: "captured",
      hotkey: { ctrl: false, alt: true, shift: false, win: false, key: "B" },
    });
  });
});

describe("hotkeyToAccelerator", () => {
  const hk = (label: string) => {
    const r = parseHotkey(label);
    if (!r.ok) throw new Error(`bad fixture: ${label}`);
    return r.hotkey;
  };

  it("maps modifiers to Electron names and keeps a letter as-is", () => {
    expect(hotkeyToAccelerator(hk("Ctrl+Shift+A"))).toBe("Control+Shift+A");
  });

  it("maps Win to Super and keeps F-keys", () => {
    expect(hotkeyToAccelerator(hk("Win+F12"))).toBe("Super+F12");
  });

  it("maps named keys to Electron accelerator tokens", () => {
    expect(hotkeyToAccelerator(hk("Ctrl+Shift+Space"))).toBe("Control+Shift+Space");
    expect(hotkeyToAccelerator(hk("Ctrl+Enter"))).toBe("Control+Return");
    expect(hotkeyToAccelerator(hk("Alt+Tab"))).toBe("Alt+Tab");
    expect(hotkeyToAccelerator(hk("Ctrl+Escape"))).toBe("Control+Escape");
    expect(hotkeyToAccelerator(hk("Ctrl+Backspace"))).toBe("Control+Backspace");
  });

  it("emits modifiers in canonical order Control, Alt, Shift, Super", () => {
    expect(
      hotkeyToAccelerator({
        ctrl: true,
        alt: true,
        shift: true,
        win: true,
        key: "A",
      }),
    ).toBe("Control+Alt+Shift+Super+A");
  });
});

describe("formatHotkey", () => {
  it("emits modifiers in canonical order (Ctrl, Alt, Shift, Win)", () => {
    expect(
      formatHotkey({ ctrl: true, alt: true, shift: true, win: true, key: "A" }),
    ).toBe("Ctrl+Alt+Shift+Win+A");
  });

  it("title-cases named keys but keeps letters and F-keys upper", () => {
    expect(
      formatHotkey({ ctrl: true, alt: false, shift: true, win: false, key: "SPACE" }),
    ).toBe("Ctrl+Shift+Space");
    expect(
      formatHotkey({ ctrl: true, alt: false, shift: false, win: false, key: "F5" }),
    ).toBe("Ctrl+F5");
    expect(
      formatHotkey({ ctrl: true, alt: false, shift: false, win: false, key: "ESCAPE" }),
    ).toBe("Ctrl+Escape");
  });

  it("round-trips parse -> format -> parse", () => {
    for (const label of ["Ctrl+Shift+Space", "Alt+J", "Win+F12", "Ctrl+Enter"]) {
      const first = parseHotkey(label);
      expect(first.ok).toBe(true);
      const reparsed = parseHotkey(formatHotkey(first.hotkey!));
      expect(reparsed).toEqual(first);
    }
  });
});
