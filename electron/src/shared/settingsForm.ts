import type { Config, SpriteConfig } from "./config";
import {
  formatHotkeyNative,
  parseHotkey,
  type Hotkey,
  type ParseError,
} from "./hotkey";
import { detectPlatform, type Platform } from "./platform";

export type IntervalResult =
  | { ok: true; value: number }
  | { ok: false; input: string };

export type HotkeyResult =
  | { ok: true; hotkey: Hotkey }
  | { ok: false; error: ParseError };

/** Which field refused to be saved. §9: if any fails the file is not
 * touched at all, so a save reports one field, not a partial write. */
export type FormError =
  | { field: "interval"; input: string }
  | { field: "hotkey"; error: ParseError }
  | { field: "sprite_x" | "sprite_y" | "card_width"; input: string };

export type ToConfigResult =
  | { ok: true; config: Config }
  | { ok: false; error: FormError };

/** Render an interval number as field text. JS `String` already drops a
 * trailing ".0" (8.0 === 8), so whole numbers print without it. */
function formatInterval(n: number): string {
  return String(n);
}

/**
 * Parse interval field text the way the spec requires: finite, strictly
 * positive. Unlike the popup's forgiving parser, the Settings UI surfaces the
 * error rather than silently defaulting — so a typo can't be saved as a valid
 * interval.
 */
export function parseIntervalMinutes(text: string): IntervalResult {
  const trimmed = text.trim();
  if (trimmed === "") return { ok: false, input: trimmed };
  const n = Number(trimmed);
  if (!Number.isFinite(n) || n <= 0) return { ok: false, input: trimmed };
  return { ok: true, value: n };
}

/** A whole number of points, optionally signed. Strict, like the interval. */
function parseInteger(text: string): number | null {
  const trimmed = text.trim();
  return /^-?\d+$/.test(trimmed) ? Number(trimmed) + 0 : null;
}

/**
 * Pure settings-form state. The render shell (Svelte) binds to the public
 * fields and asks the form questions; the form never touches disk or the OS.
 * Mirrors the reference `SettingsForm`.
 */
export class SettingsForm {
  hotkey: string;
  intervalText: string;
  autostart: boolean;
  spriteEnabled: boolean;
  spritePath: string;
  spriteXText: string;
  spriteYText: string;
  cardWidthText: string;
  private original: Config;
  private platform: Platform;

  private constructor(cfg: Config, platform: Platform) {
    this.platform = platform;
    // §9: a stored value that parses is re-spelled for this platform on load,
    // so a config written by the other build or edited by hand reads as
    // `⌃⇧Space` on a Mac. One that does not parse is shown exactly as stored —
    // it is the only copy of what the user typed, and the field is where they
    // fix it.
    this.hotkey = respell(cfg.hotkey, platform);
    this.intervalText = formatInterval(cfg.default_interval_minutes);
    this.autostart = cfg.autostart;
    this.spriteEnabled = cfg.sprite.enabled;
    this.spritePath = cfg.sprite.path;
    this.spriteXText = String(cfg.sprite.x);
    this.spriteYText = String(cfg.sprite.y);
    this.cardWidthText = String(cfg.sprite.card_width);
    this.original = { ...cfg, hotkey: this.hotkey };
  }

  static fromConfig(cfg: Config, platform: Platform = detectPlatform()): SettingsForm {
    return new SettingsForm({ ...cfg, sprite: { ...cfg.sprite } }, platform);
  }

  parsedInterval(): IntervalResult {
    return parseIntervalMinutes(this.intervalText);
  }

  parsedHotkey(): HotkeyResult {
    const result = parseHotkey(this.hotkey);
    return result.ok
      ? { ok: true, hotkey: result.hotkey }
      : { ok: false, error: result.error };
  }

  isDirty(): boolean {
    const o = this.original;
    return (
      this.hotkey !== o.hotkey ||
      this.intervalText !== formatInterval(o.default_interval_minutes) ||
      this.autostart !== o.autostart ||
      this.spriteEnabled !== o.sprite.enabled ||
      this.spritePath !== o.sprite.path ||
      this.spriteXText !== String(o.sprite.x) ||
      this.spriteYText !== String(o.sprite.y) ||
      this.cardWidthText !== String(o.sprite.card_width)
    );
  }

  /**
   * §9: both fields are validated first, and what is written is the canonical
   * spelling rather than the typed text. A hotkey the app cannot register must
   * not reach the file — the settings window is the one surface that can put
   * one there.
   */
  toConfig(): ToConfigResult {
    const hotkey = this.parsedHotkey();
    if (!hotkey.ok) return { ok: false, error: { field: "hotkey", error: hotkey.error } };
    const interval = this.parsedInterval();
    if (!interval.ok) {
      return { ok: false, error: { field: "interval", input: interval.input } };
    }
    const x = parseInteger(this.spriteXText);
    if (x === null) {
      return { ok: false, error: { field: "sprite_x", input: this.spriteXText.trim() } };
    }
    const y = parseInteger(this.spriteYText);
    if (y === null) {
      return { ok: false, error: { field: "sprite_y", input: this.spriteYText.trim() } };
    }
    const cardWidth = parseInteger(this.cardWidthText);
    if (cardWidth === null || cardWidth <= 0) {
      return { ok: false, error: { field: "card_width", input: this.cardWidthText.trim() } };
    }
    const sprite: SpriteConfig = {
      enabled: this.spriteEnabled,
      path: this.spritePath.trim(),
      x,
      y,
      card_width: cardWidth,
    };
    return {
      ok: true,
      config: {
        hotkey: formatHotkeyNative(hotkey.hotkey, this.platform),
        default_interval_minutes: interval.value,
        autostart: this.autostart,
        sprite,
      },
    };
  }

  /**
   * Rebaseline to the current (normalized) state after a successful Save. A
   * form that doesn't parse can't have been saved and is left untouched.
   */
  markClean(): void {
    const result = this.toConfig();
    if (result.ok) {
      this.original = result.config;
    } else {
      // The fields that cannot fail to parse keep their edits; the number
      // fields fall back to what is stored.
      this.original = {
        ...this.original,
        hotkey: this.hotkey.trim(),
        autostart: this.autostart,
        sprite: {
          ...this.original.sprite,
          enabled: this.spriteEnabled,
          path: this.spritePath.trim(),
        },
      };
    }
    // Rehydrate the fields so the canonical spelling sticks visibly: what the
    // user reads back is what is stored, and a just-saved form is not dirty.
    const o = this.original;
    this.hotkey = o.hotkey;
    this.intervalText = formatInterval(o.default_interval_minutes);
    this.spriteEnabled = o.sprite.enabled;
    this.spritePath = o.sprite.path;
    this.spriteXText = String(o.sprite.x);
    this.spriteYText = String(o.sprite.y);
    this.cardWidthText = String(o.sprite.card_width);
  }
}

/** A stored hotkey in this platform's spelling, or unchanged if it does not
 * parse. */
export function respell(stored: string, platform: Platform = detectPlatform()): string {
  const parsed = parseHotkey(stored);
  return parsed.ok ? formatHotkeyNative(parsed.hotkey, platform) : stored;
}
