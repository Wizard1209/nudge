import { validIntervalMinutes, type Config, type SpriteConfig } from "./config";
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

export type FormError =
  | { field: "interval"; input: string }
  | { field: "hotkey"; error: ParseError }
  | { field: "sprite_x" | "sprite_y" | "card_width"; input: string };

export type ToConfigResult =
  | { ok: true; config: Config }
  | { ok: false; error: FormError };

function formatInterval(n: number): string {
  return String(n);
}

export function parseIntervalMinutes(text: string): IntervalResult {
  const trimmed = text.trim();
  if (trimmed === "") return { ok: false, input: trimmed };
  const n = Number(trimmed);
  if (!validIntervalMinutes(n)) return { ok: false, input: trimmed };
  return { ok: true, value: n };
}

function parseInteger(text: string): number | null {
  const trimmed = text.trim();
  return /^-?\d+$/.test(trimmed) ? Number(trimmed) + 0 : null;
}

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

  markClean(): void {
    const result = this.toConfig();
    if (result.ok) {
      this.original = result.config;
    } else {
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

export function respell(stored: string, platform: Platform = detectPlatform()): string {
  const parsed = parseHotkey(stored);
  return parsed.ok ? formatHotkeyNative(parsed.hotkey, platform) : stored;
}
