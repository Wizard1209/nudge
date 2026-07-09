import { describe, it, expect } from "vitest";
import { SettingsForm } from "../../src/shared/settingsForm";
import { DEFAULT_CONFIG, type Config } from "../../src/shared/config";

const cfg = (over: Partial<Config> = {}): Config => ({
  ...DEFAULT_CONFIG,
  ...over,
});

describe("SettingsForm.fromConfig", () => {
  it("seeds fields from the config and starts clean", () => {
    const form = SettingsForm.fromConfig(
      cfg({ hotkey: "Alt+J", default_interval_minutes: 7, autostart: true }),
    );
    expect(form.hotkey).toBe("Alt+J");
    expect(form.intervalText).toBe("7");
    expect(form.autostart).toBe(true);
    expect(form.isDirty()).toBe(false);
  });

  it("renders a fractional interval without trailing-zero noise", () => {
    const form = SettingsForm.fromConfig(cfg({ default_interval_minutes: 7.5 }));
    expect(form.intervalText).toBe("7.5");
  });
});

describe("SettingsForm dirty tracking", () => {
  it("becomes dirty when a field changes and clean again on revert", () => {
    const form = SettingsForm.fromConfig(cfg({ hotkey: "Alt+J" }));
    form.hotkey = "Ctrl+K";
    expect(form.isDirty()).toBe(true);
    form.hotkey = "Alt+J";
    expect(form.isDirty()).toBe(false);
  });

  it("tracks interval and autostart edits", () => {
    const form = SettingsForm.fromConfig(cfg({ default_interval_minutes: 10 }));
    form.intervalText = "12";
    expect(form.isDirty()).toBe(true);
    form.intervalText = "10";
    expect(form.isDirty()).toBe(false);
    form.autostart = !form.autostart;
    expect(form.isDirty()).toBe(true);
  });
});

describe("SettingsForm.parsedInterval", () => {
  it("accepts finite positive numbers (with surrounding whitespace)", () => {
    const form = SettingsForm.fromConfig(cfg());
    form.intervalText = " 7.5 ";
    expect(form.parsedInterval()).toEqual({ ok: true, value: 7.5 });
    form.intervalText = "0.1";
    expect(form.parsedInterval()).toEqual({ ok: true, value: 0.1 });
  });

  it("rejects empty, non-numeric, zero and negative values", () => {
    const form = SettingsForm.fromConfig(cfg());
    for (const bad of ["", "abc", "0", "-5"]) {
      form.intervalText = bad;
      expect(form.parsedInterval().ok).toBe(false);
    }
  });
});

describe("SettingsForm.toConfig", () => {
  it("trims the hotkey and uses the parsed interval", () => {
    const form = SettingsForm.fromConfig(cfg());
    form.hotkey = "  Ctrl+Shift+A  ";
    form.intervalText = "15";
    form.autostart = true;
    const result = form.toConfig();
    expect(result).toEqual({
      ok: true,
      config: {
        hotkey: "Ctrl+Shift+A",
        default_interval_minutes: 15,
        autostart: true,
      },
    });
  });

  it("propagates an interval parse error", () => {
    const form = SettingsForm.fromConfig(cfg());
    form.intervalText = "nope";
    expect(form.toConfig().ok).toBe(false);
  });
});

describe("SettingsForm.markClean", () => {
  it("rebaselines so a saved form is no longer dirty and text is normalized", () => {
    const form = SettingsForm.fromConfig(cfg());
    form.hotkey = "  Alt+J  ";
    form.intervalText = "8";
    expect(form.isDirty()).toBe(true);
    form.markClean();
    expect(form.isDirty()).toBe(false);
    expect(form.hotkey).toBe("Alt+J");
    expect(form.intervalText).toBe("8");
  });
});
