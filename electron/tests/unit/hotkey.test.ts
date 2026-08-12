import { describe, it, expect } from "vitest";
import {
  parseHotkey,
  formatHotkey,
  formatHotkeyNative,
  hotkeyToAccelerator,
  keyCodeName,
  tokenForKeyCode,
  vocabulary,
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

  it("accepts Win aliases (Super/Meta/Cmd/Command)", () => {
    for (const alias of ["Win", "Super", "Meta", "Cmd", "Command"]) {
      expect(parseHotkey(`${alias}+J`)).toEqual({
        ok: true,
        hotkey: { ctrl: false, alt: false, shift: false, win: true, key: "J" },
      });
    }
  });

  it("accepts Alt aliases (Option/Opt)", () => {
    for (const alias of ["Alt", "Option", "Opt"]) {
      expect(parseHotkey(`${alias}+J`).hotkey?.alt).toBe(true);
    }
  });

  it("reads the modifier glyphs, with or without separators", () => {
    const alt_cmd_j = parseHotkey("Alt+Cmd+J");
    expect(parseHotkey("⌥⌘J")).toEqual(alt_cmd_j);
    expect(parseHotkey("⌥+⌘+J")).toEqual(alt_cmd_j);
    expect(parseHotkey("⌃⇧Space")).toEqual(parseHotkey("Ctrl+Shift+Space"));
    expect(parseHotkey("⊞J")).toEqual(parseHotkey("Win+J"));
  });

  it("canonicalizes named keys (Enter/Return -> ENTER, Esc -> ESCAPE)", () => {
    expect(parseHotkey("Ctrl+Enter").hotkey?.key).toBe("ENTER");
    expect(parseHotkey("Ctrl+Return").hotkey?.key).toBe("ENTER");
    expect(parseHotkey("Ctrl+Esc").hotkey?.key).toBe("ESCAPE");
    expect(parseHotkey("Ctrl+Escape").hotkey?.key).toBe("ESCAPE");
    expect(parseHotkey("Ctrl+Tab").hotkey?.key).toBe("TAB");
    expect(parseHotkey("Ctrl+Backspace").hotkey?.key).toBe("BACKSPACE");
  });

  it("reads the named-key glyphs", () => {
    const glyphs: Record<string, string> = {
      "␣": "SPACE",
      "↩": "ENTER",
      "⏎": "ENTER",
      "⌤": "ENTER",
      "⇥": "TAB",
      "⎋": "ESCAPE",
      "⌫": "BACKSPACE",
      "⌦": "DELETE",
      "↖": "HOME",
      "↘": "END",
      "⇞": "PAGEUP",
      "⇟": "PAGEDOWN",
    };
    for (const [glyph, token] of Object.entries(glyphs)) {
      expect(parseHotkey(`Ctrl+${glyph}`).hotkey?.key, glyph).toBe(token);
    }
  });

  it("parses the arrows, by word and by glyph", () => {
    const arrows: Record<string, string> = {
      Left: "LEFT",
      "←": "LEFT",
      Right: "RIGHT",
      "→": "RIGHT",
      Up: "UP",
      "↑": "UP",
      Down: "DOWN",
      "↓": "DOWN",
    };
    for (const [spelling, token] of Object.entries(arrows)) {
      expect(parseHotkey(`Ctrl+${spelling}`).hotkey?.key, spelling).toBe(token);
    }
    // The task's own example, in the Mac spelling.
    expect(parseHotkey("⌥⌘←")).toEqual({
      ok: true,
      hotkey: { ctrl: false, alt: true, shift: false, win: true, key: "LEFT" },
    });
  });

  it("parses the navigation cluster, including its short spellings", () => {
    const nav: Record<string, string> = {
      Home: "HOME",
      End: "END",
      PageUp: "PAGEUP",
      PgUp: "PAGEUP",
      PageDown: "PAGEDOWN",
      PgDn: "PAGEDOWN",
      Insert: "INSERT",
      Ins: "INSERT",
      Delete: "DELETE",
      Del: "DELETE",
    };
    for (const [spelling, token] of Object.entries(nav)) {
      expect(parseHotkey(`Ctrl+${spelling}`).hotkey?.key, spelling).toBe(token);
    }
  });

  it("parses every symbol cap", () => {
    for (const cap of ["-", "=", "[", "]", "\\", ";", "'", ",", ".", "/", "`"]) {
      expect(parseHotkey(`Ctrl+${cap}`).hotkey?.key, cap).toBe(cap);
    }
  });

  it("names a symbol cap by its unshifted legend, so ⇧/ is the spelling of ?", () => {
    expect(parseHotkey("⇧/")).toEqual({
      ok: true,
      hotkey: { ctrl: false, alt: false, shift: true, win: false, key: "/" },
    });
    // The shifted twin is not a second name for the same cap.
    expect(parseHotkey("Ctrl+?")).toEqual({
      ok: false,
      error: { kind: "unknownToken", token: "?" },
    });
  });

  it("treats + as the separator only, so the cap next to Backspace is =", () => {
    expect(parseHotkey("Ctrl+=").hotkey?.key).toBe("=");
    expect(parseHotkey("Ctrl++")).toEqual({
      ok: false,
      error: { kind: "unknownToken", token: "" },
    });
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

  it("collapses repeated modifiers", () => {
    expect(parseHotkey("Ctrl+Ctrl+A")).toEqual(parseHotkey("Ctrl+A"));
  });

  it("rejects an unsupported key token", () => {
    expect(parseHotkey("Ctrl+Numpad5")).toEqual({
      ok: false,
      error: { kind: "unknownToken", token: "Numpad5" },
    });
    expect(parseHotkey("Ctrl+CapsLock")).toEqual({
      ok: false,
      error: { kind: "unknownToken", token: "CapsLock" },
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
    expect(parseHotkey("⌥⌘")).toEqual({
      ok: false,
      error: { kind: "missingKey" },
    });
  });

  it("rejects a key that types something when nothing is held", () => {
    // Registration would take every one of these and then own the key on the
    // whole machine (spec §5).
    for (const s of ["J", "5", "Space", "Enter", "Home", "←", "/"]) {
      const key = parseHotkey(`Ctrl+${s}`).hotkey?.key;
      expect(parseHotkey(s), s).toEqual({
        ok: false,
        error: { kind: "missingModifier", key },
      });
    }
  });

  it("lets the F-row stand alone", () => {
    for (const s of ["F1", "F13", "F24"]) {
      const result = parseHotkey(s);
      expect(result.ok).toBe(true);
      expect(result.hotkey).toEqual({
        ctrl: false,
        alt: false,
        shift: false,
        win: false,
        key: s,
      });
    }
  });

  it("rejects more than one key", () => {
    expect(parseHotkey("Ctrl+A+B")).toEqual({
      ok: false,
      error: { kind: "multipleKeys" },
    });
    expect(parseHotkey("⌘←→")).toEqual({
      ok: false,
      // "←→" is one segment and neither arrow is a modifier glyph, so the
      // token is unknown rather than two keys.
      error: { kind: "unknownToken", token: "←→" },
    });
  });

  it("rejects an empty segment (Ctrl++A)", () => {
    expect(parseHotkey("Ctrl++A")).toEqual({
      ok: false,
      error: { kind: "unknownToken", token: "" },
    });
  });
});

describe("formatHotkey", () => {
  const hk = (label: string) => {
    const r = parseHotkey(label);
    if (!r.ok) throw new Error(`bad fixture: ${label}`);
    return r.hotkey;
  };

  it("emits modifiers in canonical order (Ctrl, Alt, Shift, Win)", () => {
    expect(
      formatHotkey(
        { ctrl: true, alt: true, shift: true, win: true, key: "A" },
        "windows",
      ),
    ).toBe("Ctrl+Alt+Shift+Win+A");
  });

  it("names the fourth modifier Cmd on macOS and Win elsewhere", () => {
    expect(formatHotkey(hk("Cmd+J"), "macos")).toBe("Cmd+J");
    expect(formatHotkey(hk("Cmd+J"), "windows")).toBe("Win+J");
  });

  it("title-cases named keys but keeps letters, digits and caps as written", () => {
    expect(formatHotkey(hk("Ctrl+Shift+Space"), "windows")).toBe("Ctrl+Shift+Space");
    expect(formatHotkey(hk("Ctrl+F5"), "windows")).toBe("Ctrl+F5");
    expect(formatHotkey(hk("Ctrl+Esc"), "windows")).toBe("Ctrl+Escape");
    expect(formatHotkey(hk("Ctrl+PgDn"), "windows")).toBe("Ctrl+PageDown");
    expect(formatHotkey(hk("Ctrl+←"), "windows")).toBe("Ctrl+Left");
    expect(formatHotkey(hk("Ctrl+/"), "windows")).toBe("Ctrl+/");
  });

  it("round-trips parse -> format -> parse", () => {
    const labels = [
      "Ctrl+Shift+Space",
      "Alt+J",
      "Win+F12",
      "Ctrl+Enter",
      "Ctrl+Alt+PageUp",
      "Shift+/",
      "Ctrl+Shift+←",
      "Alt+Delete",
    ];
    for (const label of labels) {
      const first = parseHotkey(label);
      expect(first.ok, label).toBe(true);
      for (const platform of ["macos", "windows"] as const) {
        expect(parseHotkey(formatHotkey(first.hotkey!, platform)), label).toEqual(first);
        expect(parseHotkey(formatHotkeyNative(first.hotkey!, platform)), label).toEqual(
          first,
        );
      }
    }
  });
});

describe("formatHotkeyNative", () => {
  const hk = (label: string) => {
    const r = parseHotkey(label);
    if (!r.ok) throw new Error(`bad fixture: ${label}`);
    return r.hotkey;
  };

  it("writes the macOS symbol form", () => {
    expect(formatHotkeyNative(hk("Ctrl+Shift+Space"), "macos")).toBe("⌃⇧Space");
    expect(formatHotkeyNative(hk("Alt+Cmd+J"), "macos")).toBe("⌥⌘J");
    expect(formatHotkeyNative(hk("Alt+Cmd+Left"), "macos")).toBe("⌥⌘←");
    expect(formatHotkeyNative(hk("Shift+/"), "macos")).toBe("⇧/");
  });

  it("keeps the word where a Mac keycap carries one (Space, Insert)", () => {
    expect(formatHotkeyNative(hk("Ctrl+Space"), "macos")).toBe("⌃Space");
    expect(formatHotkeyNative(hk("Ctrl+Insert"), "macos")).toBe("⌃Insert");
    expect(formatHotkeyNative(hk("Ctrl+F5"), "macos")).toBe("⌃F5");
  });

  it("uses the keycap glyph where a Mac has one", () => {
    expect(formatHotkeyNative(hk("Ctrl+Enter"), "macos")).toBe("⌃↩");
    expect(formatHotkeyNative(hk("Ctrl+Backspace"), "macos")).toBe("⌃⌫");
    expect(formatHotkeyNative(hk("Ctrl+Delete"), "macos")).toBe("⌃⌦");
    expect(formatHotkeyNative(hk("Ctrl+PageUp"), "macos")).toBe("⌃⇞");
    expect(formatHotkeyNative(hk("Ctrl+Escape"), "macos")).toBe("⌃⎋");
  });

  it("is the word form on Windows", () => {
    expect(formatHotkeyNative(hk("⌥⌘J"), "windows")).toBe("Alt+Win+J");
  });
});

describe("vocabulary", () => {
  it("names the platform's modifiers and F-row, and lists the symbol caps", () => {
    expect(vocabulary("macos")).toContain("⌃ ⌥ ⇧ ⌘");
    expect(vocabulary("macos")).toContain("F1–F20");
    expect(vocabulary("windows")).toContain("Ctrl, Alt, Shift, Win");
    expect(vocabulary("windows")).toContain("F1–F24");
    for (const line of [vocabulary("macos"), vocabulary("windows")]) {
      expect(line).toContain("← ↑ → ↓");
      expect(line).toContain("- = [ ] \\ ; ' , . / `");
    }
  });
});

describe("tokenForKeyCode", () => {
  it("maps the physical letter and digit caps", () => {
    expect(tokenForKeyCode("KeyJ")).toBe("J");
    expect(tokenForKeyCode("Digit5")).toBe("5");
  });

  it("maps the named keys, the arrows and the navigation cluster", () => {
    expect(tokenForKeyCode("Space")).toBe("SPACE");
    expect(tokenForKeyCode("Enter")).toBe("ENTER");
    expect(tokenForKeyCode("Escape")).toBe("ESCAPE");
    expect(tokenForKeyCode("ArrowLeft")).toBe("LEFT");
    expect(tokenForKeyCode("PageDown")).toBe("PAGEDOWN");
    expect(tokenForKeyCode("F13")).toBe("F13");
  });

  it("maps every symbol cap by position", () => {
    const caps: Record<string, string> = {
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
    for (const [code, token] of Object.entries(caps)) {
      expect(tokenForKeyCode(code), code).toBe(token);
    }
  });

  it("has no token for the keypad or the lock keys", () => {
    for (const code of ["Numpad5", "NumpadEnter", "NumLock", "CapsLock"]) {
      expect(tokenForKeyCode(code), code).toBeNull();
    }
  });
});

describe("keyCodeName", () => {
  it("names a keypad key as the keypad, never as the digit row", () => {
    expect(keyCodeName("Numpad5")).toBe("Keypad 5");
    expect(keyCodeName("Digit5")).toBe("5");
    expect(keyCodeName("NumpadEnter")).toBe("Keypad Enter");
    expect(keyCodeName("NumpadAdd")).toBe("Keypad +");
  });

  it("names a key inside the grammar by its §5 label", () => {
    expect(keyCodeName("KeyJ")).toBe("J");
    expect(keyCodeName("ArrowLeft")).toBe("Left");
    expect(keyCodeName("PageDown")).toBe("PageDown");
    expect(keyCodeName("Slash")).toBe("/");
  });

  it("falls back to the raw code so a refusal still says which key", () => {
    expect(keyCodeName("CapsLock")).toBe("Caps Lock");
    expect(keyCodeName("MediaPlayPause")).toBe("MediaPlayPause");
    expect(keyCodeName("")).toBe("Unknown key");
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

  it("maps the arrows and the navigation cluster", () => {
    expect(hotkeyToAccelerator(hk("Ctrl+←"))).toBe("Control+Left");
    expect(hotkeyToAccelerator(hk("Ctrl+↑"))).toBe("Control+Up");
    expect(hotkeyToAccelerator(hk("Ctrl+PgDn"))).toBe("Control+PageDown");
    expect(hotkeyToAccelerator(hk("Ctrl+Del"))).toBe("Control+Delete");
    expect(hotkeyToAccelerator(hk("Ctrl+Ins"))).toBe("Control+Insert");
    expect(hotkeyToAccelerator(hk("Ctrl+Home"))).toBe("Control+Home");
  });

  it("passes a symbol cap through unchanged", () => {
    expect(hotkeyToAccelerator(hk("Ctrl+/"))).toBe("Control+/");
    expect(hotkeyToAccelerator(hk("Ctrl+="))).toBe("Control+=");
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
