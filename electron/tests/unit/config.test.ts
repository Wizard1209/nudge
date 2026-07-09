import { describe, it, expect, beforeEach, afterEach } from "vitest";
import { mkdtempSync, rmSync, writeFileSync, existsSync, readdirSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  loadConfig,
  saveConfig,
  diffConfig,
  DEFAULT_CONFIG,
  type Config,
} from "../../src/electron/config";

let dir: string;
let path: string;

beforeEach(() => {
  dir = mkdtempSync(join(tmpdir(), "nudge-config-"));
  path = join(dir, "config.json");
});

afterEach(() => {
  rmSync(dir, { recursive: true, force: true });
});

describe("saveConfig / loadConfig", () => {
  it("round-trips a config through disk", () => {
    const config: Config = {
      hotkey: "Ctrl+Alt+J",
      default_interval_minutes: 7,
      autostart: true,
    };
    saveConfig(path, config);
    expect(loadConfig(path)).toEqual({ config, error: null });
  });

  it("returns defaults with no error when the file is missing", () => {
    expect(loadConfig(path)).toEqual({ config: DEFAULT_CONFIG, error: null });
  });

  it("returns defaults with an error when the JSON is malformed", () => {
    writeFileSync(path, "{ not json", "utf-8");
    const result = loadConfig(path);
    expect(result.config).toEqual(DEFAULT_CONFIG);
    expect(result.error).toBeInstanceOf(Error);
  });

  it("fills missing fields from defaults and tolerates unknown fields", () => {
    writeFileSync(
      path,
      JSON.stringify({ default_interval_minutes: 5, future_field: "ignored" }),
      "utf-8",
    );
    expect(loadConfig(path)).toEqual({
      config: {
        hotkey: "Ctrl+Shift+Space",
        default_interval_minutes: 5,
        autostart: false,
      },
      error: null,
    });
  });

  it("treats an empty object as all defaults", () => {
    writeFileSync(path, "{}", "utf-8");
    expect(loadConfig(path)).toEqual({ config: DEFAULT_CONFIG, error: null });
  });

  it("falls back to the default interval when the stored value is invalid", () => {
    for (const bad of [0, -3, "7"]) {
      writeFileSync(path, JSON.stringify({ default_interval_minutes: bad }), "utf-8");
      expect(loadConfig(path).config.default_interval_minutes).toBe(10);
    }
  });
});

describe("diffConfig", () => {
  const base: Config = {
    hotkey: "Ctrl+Shift+Space",
    default_interval_minutes: 10,
    autostart: false,
  };

  it("reports no changes for identical configs", () => {
    expect(diffConfig(base, { ...base })).toEqual({
      hotkey: false,
      interval: false,
      autostart: false,
    });
  });

  it("detects a hotkey change in isolation", () => {
    expect(diffConfig(base, { ...base, hotkey: "Alt+J" })).toEqual({
      hotkey: true,
      interval: false,
      autostart: false,
    });
  });

  it("detects an interval change in isolation", () => {
    expect(diffConfig(base, { ...base, default_interval_minutes: 5 })).toEqual({
      hotkey: false,
      interval: true,
      autostart: false,
    });
  });

  it("detects an autostart change in isolation", () => {
    expect(diffConfig(base, { ...base, autostart: true })).toEqual({
      hotkey: false,
      interval: false,
      autostart: true,
    });
  });
});

describe("saveConfig atomicity", () => {
  it("creates parent dirs and leaves no temp file behind", () => {
    const nested = join(dir, "a", "b", "config.json");
    saveConfig(nested, DEFAULT_CONFIG);
    expect(existsSync(nested)).toBe(true);
    expect(readdirSync(join(dir, "a", "b"))).toEqual(["config.json"]);
  });
});
