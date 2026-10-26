/**
 * Pure config shape + coercion. No I/O — safe to import from the browser
 * Settings sub-app as well as the Electron main process. The node-only file
 * read/write/path lives in `src/electron/config.ts`, which re-exports these.
 */

import { CARD_WIDTH } from "./sprite";

/** Optional PNG drawn 1:1 in front of the popup card. */
export interface SpriteConfig {
  enabled: boolean;
  /** Absolute path to the PNG. */
  path: string;
  /** Sprite top-left relative to the card top-left, px; may be negative. */
  x: number;
  y: number;
  /** Card width while the sprite is on, px. Ignored when off. */
  card_width: number;
}

export interface Config {
  /** Global hotkey label in the §5 format, e.g. "Ctrl+Shift+Space". */
  hotkey: string;
  /** Default "next nudge in" interval, in minutes. Finite and > 0. */
  default_interval_minutes: number;
  /** Whether the app is registered to launch with the OS session. */
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

/** Finite number → nearest integer (never -0); anything else → null. */
function roundedInt(value: unknown): number | null {
  if (typeof value !== "number" || !Number.isFinite(value)) return null;
  return Math.round(value) + 0;
}

/** Same forgiving rule as the top level: each bad field falls back alone. */
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

/**
 * Coerce arbitrary parsed JSON into a valid Config. Each field that is absent
 * or invalid silently falls back to its default — a hand-edited or
 * partially-written config never crashes the app (spec §9: forgiving load).
 */
export function normalize(raw: unknown): Config {
  const obj = (raw ?? {}) as Record<string, unknown>;
  return {
    hotkey:
      typeof obj.hotkey === "string" && obj.hotkey.trim() !== ""
        ? obj.hotkey
        : DEFAULT_CONFIG.hotkey,
    default_interval_minutes:
      typeof obj.default_interval_minutes === "number" &&
      Number.isFinite(obj.default_interval_minutes) &&
      obj.default_interval_minutes > 0
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

/**
 * Per-field change report between two configs. The file watcher uses this to
 * decide what to re-apply live — per spec §9 only a hotkey change has a live
 * effect; interval/autostart are cached for the next nudge / next launch. A
 * sprite change is applied while the popup is hidden.
 */
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
