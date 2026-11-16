import { describe, it, expect, beforeEach, afterEach } from "vitest";
import {
  mkdtempSync,
  rmSync,
  writeFileSync,
  readFileSync,
  mkdirSync,
  existsSync,
  readdirSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  loadConfig,
  saveConfig,
  diffConfig,
  normalize,
  DEFAULT_CONFIG,
  configDirFor,
  migrateLegacyConfig,
  type Config,
} from "../../src/electron/config";

const SPRITE_OFF = { enabled: false, path: "", x: 0, y: 0, card_width: 480 };

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
      sprite: { enabled: true, path: "C:\\art\\sprite.png", x: -41, y: -277, card_width: 679 },
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
        sprite: SPRITE_OFF,
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

describe("normalize: sprite block", () => {
  it("defaults to a disabled sprite when absent", () => {
    expect(DEFAULT_CONFIG.sprite).toEqual(SPRITE_OFF);
    expect(normalize({}).sprite).toEqual(SPRITE_OFF);
  });

  it("keeps a valid block, negatives included", () => {
    const sprite = { enabled: true, path: "/a/b.png", x: -41, y: -277, card_width: 679 };
    expect(normalize({ sprite }).sprite).toEqual(sprite);
  });

  it("falls back per field on bad types", () => {
    expect(
      normalize({
        sprite: { enabled: "yes", path: 7, x: "3", y: null, card_width: "679" },
      }).sprite,
    ).toEqual(SPRITE_OFF);
  });

  it("falls back to defaults when the block is not an object", () => {
    for (const bad of [null, true, 5, "on", [1, 2]]) {
      expect(normalize({ sprite: bad }).sprite).toEqual(SPRITE_OFF);
    }
  });

  it("rounds non-integer coordinates and card width", () => {
    expect(
      normalize({ sprite: { x: -40.6, y: 12.4, card_width: 679.5 } }).sprite,
    ).toMatchObject({ x: -41, y: 12, card_width: 680 });
  });

  it("never yields a negative zero", () => {
    expect(Object.is(normalize({ sprite: { x: -0.2 } }).sprite.x, 0)).toBe(true);
  });

  it("rejects a non-positive or non-finite card width", () => {
    for (const bad of [0, -5, 0.4, Infinity, NaN]) {
      expect(normalize({ sprite: { card_width: bad } }).sprite.card_width).toBe(480);
    }
  });

  it("rejects non-finite coordinates", () => {
    expect(normalize({ sprite: { x: Infinity, y: NaN } }).sprite).toMatchObject({ x: 0, y: 0 });
  });
});

describe("diffConfig", () => {
  const base: Config = {
    hotkey: "Ctrl+Shift+Space",
    default_interval_minutes: 10,
    autostart: false,
    sprite: SPRITE_OFF,
  };

  it("reports no changes for identical configs", () => {
    expect(diffConfig(base, { ...base })).toEqual({
      hotkey: false,
      interval: false,
      autostart: false,
      sprite: false,
    });
  });

  it("detects a hotkey change in isolation", () => {
    expect(diffConfig(base, { ...base, hotkey: "Alt+J" })).toEqual({
      hotkey: true,
      interval: false,
      autostart: false,
      sprite: false,
    });
  });

  it("detects an interval change in isolation", () => {
    expect(diffConfig(base, { ...base, default_interval_minutes: 5 })).toEqual({
      hotkey: false,
      interval: true,
      autostart: false,
      sprite: false,
    });
  });

  it("detects an autostart change in isolation", () => {
    expect(diffConfig(base, { ...base, autostart: true })).toEqual({
      hotkey: false,
      interval: false,
      autostart: true,
      sprite: false,
    });
  });
});

describe("diffConfig: sprite", () => {
  it("flags any sprite field change", () => {
    const base = normalize({});
    for (const change of [
      { enabled: true },
      { path: "/x.png" },
      { x: 1 },
      { y: -1 },
      { card_width: 600 },
    ]) {
      const next = { ...base, sprite: { ...base.sprite, ...change } };
      expect(diffConfig(base, next).sprite).toBe(true);
    }
    expect(diffConfig(base, { ...base, sprite: { ...base.sprite } }).sprite).toBe(false);
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

describe("configDirFor", () => {
  it("puts the config under appData/Nudge, not beside the journal", () => {
    const appData = "C:\\Users\\u\\AppData\\Roaming";
    expect(configDirFor(appData)).toBe(join(appData, "Nudge"));
  });
});

describe("migrateLegacyConfig", () => {
  let legacy: string;
  let dest: string;

  beforeEach(() => {
    legacy = join(dir, "Documents", "Nudge", "config.json");
    dest = join(dir, "AppSupport", "Nudge", "config.json");
    mkdirSync(join(dir, "Documents", "Nudge"), { recursive: true });
  });

  it("moves the legacy file when the destination does not exist", () => {
    writeFileSync(legacy, '{"hotkey":"Alt+J"}');
    migrateLegacyConfig(legacy, dest);
    expect(existsSync(legacy)).toBe(false);
    expect(readFileSync(dest, "utf-8")).toBe('{"hotkey":"Alt+J"}');
  });

  it("leaves both files alone when the destination exists", () => {
    writeFileSync(legacy, "old");
    mkdirSync(join(dir, "AppSupport", "Nudge"), { recursive: true });
    writeFileSync(dest, "new");
    migrateLegacyConfig(legacy, dest);
    expect(readFileSync(legacy, "utf-8")).toBe("old");
    expect(readFileSync(dest, "utf-8")).toBe("new");
  });

  it("does nothing, and creates nothing, when there is no legacy file", () => {
    migrateLegacyConfig(legacy, dest);
    expect(existsSync(dest)).toBe(false);
    expect(existsSync(join(dir, "AppSupport"))).toBe(false);
  });

  it("does nothing when legacy and destination are the same path", () => {
    writeFileSync(legacy, "x");
    migrateLegacyConfig(legacy, legacy);
    expect(readFileSync(legacy, "utf-8")).toBe("x");
  });
});
