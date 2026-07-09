/**
 * Pure config shape + coercion. No I/O — safe to import from the browser
 * Settings sub-app as well as the Electron main process. The node-only file
 * read/write/path lives in `src/electron/config.ts`, which re-exports these.
 */

export interface Config {
  /** Global hotkey label in the §5 format, e.g. "Ctrl+Shift+Space". */
  hotkey: string;
  /** Default "next nudge in" interval, in minutes. Finite and > 0. */
  default_interval_minutes: number;
  /** Whether the app is registered to launch with the OS session. */
  autostart: boolean;
}

export const DEFAULT_CONFIG: Config = {
  hotkey: "Ctrl+Shift+Space",
  default_interval_minutes: 10,
  autostart: false,
};

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
  };
}

export interface ConfigDiff {
  hotkey: boolean;
  interval: boolean;
  autostart: boolean;
}

/**
 * Per-field change report between two configs. The file watcher uses this to
 * decide what to re-apply live — per spec §9 only a hotkey change has a live
 * effect; interval/autostart are cached for the next nudge / next launch.
 */
export function diffConfig(prev: Config, next: Config): ConfigDiff {
  return {
    hotkey: prev.hotkey !== next.hotkey,
    interval: prev.default_interval_minutes !== next.default_interval_minutes,
    autostart: prev.autostart !== next.autostart,
  };
}
