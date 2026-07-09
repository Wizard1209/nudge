import type { Config } from "./config";

export type IntervalResult =
  | { ok: true; value: number }
  | { ok: false; input: string };

export type ToConfigResult =
  | { ok: true; config: Config }
  | { ok: false; input: string };

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

/**
 * Pure settings-form state. The render shell (Svelte) binds to the public
 * fields and asks the form questions; the form never touches disk or the OS.
 * Mirrors the reference `SettingsForm`.
 */
export class SettingsForm {
  hotkey: string;
  intervalText: string;
  autostart: boolean;
  private original: Config;

  private constructor(cfg: Config) {
    this.hotkey = cfg.hotkey;
    this.intervalText = formatInterval(cfg.default_interval_minutes);
    this.autostart = cfg.autostart;
    this.original = cfg;
  }

  static fromConfig(cfg: Config): SettingsForm {
    return new SettingsForm({ ...cfg });
  }

  parsedInterval(): IntervalResult {
    return parseIntervalMinutes(this.intervalText);
  }

  isDirty(): boolean {
    return (
      this.hotkey !== this.original.hotkey ||
      this.intervalText !==
        formatInterval(this.original.default_interval_minutes) ||
      this.autostart !== this.original.autostart
    );
  }

  toConfig(): ToConfigResult {
    const interval = this.parsedInterval();
    if (!interval.ok) return { ok: false, input: interval.input };
    return {
      ok: true,
      config: {
        hotkey: this.hotkey.trim(),
        default_interval_minutes: interval.value,
        autostart: this.autostart,
      },
    };
  }

  /** Rebaseline to the current (normalized) state after a successful Save. */
  markClean(): void {
    const interval = this.parsedInterval();
    this.original = {
      hotkey: this.hotkey.trim(),
      default_interval_minutes: interval.ok
        ? interval.value
        : this.original.default_interval_minutes,
      autostart: this.autostart,
    };
    // Rehydrate the fields so trimming/normalization sticks visibly.
    this.hotkey = this.original.hotkey;
    this.intervalText = formatInterval(this.original.default_interval_minutes);
  }
}
