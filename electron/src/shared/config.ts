import { CARD_WIDTH } from "./sprite";

export interface SpriteConfig {
  enabled: boolean;
  path: string;
  x: number;
  y: number;
  card_width: number;
}

export interface Config {
  hotkey: string;
  default_interval_minutes: number;
  autostart: boolean;
  sprite: SpriteConfig;
}

export const DEFAULT_SPRITE: SpriteConfig = {
  enabled: false,
  path: "",
  x: 0,
  y: 0,
  card_width: CARD_WIDTH,
};

export const DEFAULT_CONFIG: Config = {
  hotkey: "Ctrl+Shift+Space",
  default_interval_minutes: 10,
  autostart: false,
  sprite: DEFAULT_SPRITE,
};

export const MAX_INTERVAL_MINUTES = 1_000_000;

export function validIntervalMinutes(minutes: number): boolean {
  return minutes > 0 && minutes <= MAX_INTERVAL_MINUTES;
}

function roundedInt(value: unknown): number | null {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  return Math.round(value) + 0;
}

export function normalizeSprite(raw: unknown): SpriteConfig {
  const obj =
    raw && typeof raw === "object" && !Array.isArray(raw)
      ? (raw as Record<string, unknown>)
      : {};
  const cardWidth = roundedInt(obj.card_width);
  return {
    enabled: typeof obj.enabled === "boolean" ? obj.enabled : DEFAULT_SPRITE.enabled,
    path: typeof obj.path === "string" ? obj.path : DEFAULT_SPRITE.path,
    x: roundedInt(obj.x) ?? DEFAULT_SPRITE.x,
    y: roundedInt(obj.y) ?? DEFAULT_SPRITE.y,
    card_width: cardWidth !== null && cardWidth > 0 ? cardWidth : DEFAULT_SPRITE.card_width,
  };
}

export function normalize(raw: unknown): Config {
  const obj = (raw ?? {}) as Record<string, unknown>;
  return {
    hotkey:
      typeof obj.hotkey === "string" && obj.hotkey.trim() !== ""
        ? obj.hotkey
        : DEFAULT_CONFIG.hotkey,
    default_interval_minutes:
      typeof obj.default_interval_minutes === "number" &&
      validIntervalMinutes(obj.default_interval_minutes)
        ? obj.default_interval_minutes
        : DEFAULT_CONFIG.default_interval_minutes,
    autostart:
      typeof obj.autostart === "boolean"
        ? obj.autostart
        : DEFAULT_CONFIG.autostart,
    sprite: normalizeSprite(obj.sprite),
  };
}

export interface ConfigDiff {
  hotkey: boolean;
  interval: boolean;
  autostart: boolean;
  sprite: boolean;
}

export function diffConfig(prev: Config, next: Config): ConfigDiff {
  return {
    hotkey: prev.hotkey !== next.hotkey,
    interval: prev.default_interval_minutes !== next.default_interval_minutes,
    autostart: prev.autostart !== next.autostart,
    sprite: (Object.keys(prev.sprite) as Array<keyof SpriteConfig>).some(
      (k) => prev.sprite[k] !== next.sprite[k],
    ),
  };
}
