import { describe, it, expect } from "vitest";
import { SettingsForm, respell } from "../../src/shared/settingsForm";
import { DEFAULT_CONFIG, type Config } from "../../src/shared/config";

const cfg = (over: Partial<Config> = {}): Config => ({
  ...DEFAULT_CONFIG,
  ...over,
});

// Every form here names its platform: §5 has one canonical spelling per
// platform, so a form that read the host's would assert a different string on
// a Mac than on a PC.
const form = (over: Partial<Config> = {}, platform: "macos" | "windows" = "windows") =>
  SettingsForm.fromConfig(cfg(over), platform);

describe("SettingsForm.fromConfig", () => {
  it("seeds fields from the config and starts clean", () => {
    const f = form({ hotkey: "Alt+J", default_interval_minutes: 7, autostart: true });
    expect(f.hotkey).toBe("Alt+J");
    expect(f.intervalText).toBe("7");
    expect(f.autostart).toBe(true);
    expect(f.isDirty()).toBe(false);
  });

  it("re-spells a stored hotkey for this platform, and stays clean (§9)", () => {
    const mac = form({ hotkey: "Alt+Cmd+J" }, "macos");
    expect(mac.hotkey).toBe("⌥⌘J");
    expect(mac.isDirty()).toBe(false);

    const win = form({ hotkey: "⌥⌘J" }, "windows");
    expect(win.hotkey).toBe("Alt+Win+J");
    expect(win.isDirty()).toBe(false);
  });

  it("shows a value that does not parse exactly as stored", () => {
    // It is the only copy of what the user typed, and the field is where they
    // fix it.
    expect(form({ hotkey: "Ctrl+Nonsense" }, "macos").hotkey).toBe("Ctrl+Nonsense");
  });

  it("renders a fractional interval without trailing-zero noise", () => {
    expect(form({ default_interval_minutes: 7.5 }).intervalText).toBe("7.5");
  });
});

describe("SettingsForm dirty tracking", () => {
  it("becomes dirty when a field changes and clean again on revert", () => {
    const f = form({ hotkey: "Alt+J" });
    f.hotkey = "Ctrl+K";
    expect(f.isDirty()).toBe(true);
    f.hotkey = "Alt+J";
    expect(f.isDirty()).toBe(false);
  });

  it("tracks interval and autostart edits", () => {
    const f = form({ default_interval_minutes: 10 });
    f.intervalText = "12";
    expect(f.isDirty()).toBe(true);
    f.intervalText = "10";
    expect(f.isDirty()).toBe(false);
    f.autostart = !f.autostart;
    expect(f.isDirty()).toBe(true);
  });
});

describe("SettingsForm.parsedInterval", () => {
  it("accepts finite positive numbers (with surrounding whitespace)", () => {
    const f = form();
    f.intervalText = " 7.5 ";
    expect(f.parsedInterval()).toEqual({ ok: true, value: 7.5 });
    f.intervalText = "0.1";
    expect(f.parsedInterval()).toEqual({ ok: true, value: 0.1 });
  });

  it("rejects empty, non-numeric, zero and negative values", () => {
    const f = form();
    for (const bad of ["", "abc", "0", "-5"]) {
      f.intervalText = bad;
      expect(f.parsedInterval().ok).toBe(false);
    }
  });
});

describe("SettingsForm.toConfig", () => {
  it("writes the canonical spelling, not the typed text", () => {
    const f = form({}, "macos");
    f.hotkey = "  alt + cmd + j  ";
    f.intervalText = "15";
    f.autostart = true;
    expect(f.toConfig()).toEqual({
      ok: true,
      config: {
        hotkey: "⌥⌘J",
        default_interval_minutes: 15,
        autostart: true,
        sprite: DEFAULT_CONFIG.sprite,
      },
    });
  });

  it("refuses a hotkey the app cannot register (§9: nothing is written)", () => {
    const f = form();
    f.hotkey = "Ctrl+Numpad5";
    expect(f.toConfig()).toEqual({
      ok: false,
      error: { field: "hotkey", error: { kind: "unknownToken", token: "Numpad5" } },
    });
    f.hotkey = "J";
    expect(f.toConfig()).toEqual({
      ok: false,
      error: { field: "hotkey", error: { kind: "missingModifier", key: "J" } },
    });
  });

  it("propagates an interval parse error", () => {
    const f = form();
    f.intervalText = "nope";
    expect(f.toConfig()).toEqual({
      ok: false,
      error: { field: "interval", input: "nope" },
    });
  });
});

describe("SettingsForm.markClean", () => {
  it("rebaselines so a saved form is no longer dirty and text is normalized", () => {
    const f = form();
    f.hotkey = "  alt+j  ";
    f.intervalText = "8";
    expect(f.isDirty()).toBe(true);
    f.markClean();
    expect(f.isDirty()).toBe(false);
    expect(f.hotkey).toBe("Alt+J");
    expect(f.intervalText).toBe("8");
  });
});

describe("respell", () => {
  it("puts a stored hotkey in this platform's spelling", () => {
    expect(respell("Ctrl+Shift+Space", "macos")).toBe("⌃⇧Space");
    expect(respell("⌃⇧Space", "windows")).toBe("Ctrl+Shift+Space");
  });

  it("leaves a value that does not parse alone", () => {
    expect(respell("nonsense", "macos")).toBe("nonsense");
  });
});

describe("SettingsForm sprite fields", () => {
  const sprite = { enabled: true, path: "C:\\art\\s.png", x: -41, y: -277, card_width: 679 };

  it("seeds the sprite fields from the config", () => {
    const form = SettingsForm.fromConfig(cfg({ sprite }), "windows");
    expect(form.spriteEnabled).toBe(true);
    expect(form.spritePath).toBe("C:\\art\\s.png");
    expect([form.spriteXText, form.spriteYText, form.cardWidthText]).toEqual(["-41", "-277", "679"]);
    expect(form.isDirty()).toBe(false);
  });

  it("round-trips the sprite block through toConfig, trimming text", () => {
    const form = SettingsForm.fromConfig(cfg(), "windows");
    form.spriteEnabled = true;
    form.spritePath = "  C:\\art\\s.png ";
    form.spriteXText = " -41";
    form.spriteYText = "-277 ";
    form.cardWidthText = "679";
    const result = form.toConfig();
    expect(result.ok && result.config.sprite).toEqual(sprite);
  });

  it("tracks every sprite field as dirty", () => {
    const edits: Array<(f: SettingsForm) => void> = [
      (f) => (f.spriteEnabled = true),
      (f) => (f.spritePath = "/x.png"),
      (f) => (f.spriteXText = "5"),
      (f) => (f.spriteYText = "5"),
      (f) => (f.cardWidthText = "500"),
    ];
    for (const edit of edits) {
      const form = SettingsForm.fromConfig(cfg(), "windows");
      edit(form);
      expect(form.isDirty()).toBe(true);
    }
  });

  it("rejects non-integer coordinates, naming the field", () => {
    for (const [field, key] of [
      ["sprite_x", "spriteXText"],
      ["sprite_y", "spriteYText"],
    ] as const) {
      for (const bad of ["", "1.5", "abc", "1e3x"]) {
        const form = SettingsForm.fromConfig(cfg(), "windows");
        form[key] = bad;
        expect(form.toConfig()).toEqual({ ok: false, error: { field, input: bad.trim() } });
      }
    }
  });

  it("rejects a card width that is not a positive integer", () => {
    for (const bad of ["", "0", "-480", "480.5", "wide"]) {
      const form = SettingsForm.fromConfig(cfg(), "windows");
      form.cardWidthText = bad;
      expect(form.toConfig()).toEqual({
        ok: false,
        error: { field: "card_width", input: bad.trim() },
      });
    }
  });

  it("validates the number fields even while the sprite is off", () => {
    const form = SettingsForm.fromConfig(cfg(), "windows");
    form.spriteXText = "x";
    expect(form.toConfig().ok).toBe(false);
  });

  it("markClean rebaselines the sprite fields", () => {
    const form = SettingsForm.fromConfig(cfg(), "windows");
    form.spriteEnabled = true;
    form.spritePath = " /x.png ";
    form.spriteXText = " 7 ";
    form.markClean();
    expect(form.isDirty()).toBe(false);
    expect([form.spritePath, form.spriteXText]).toEqual(["/x.png", "7"]);
  });
});
