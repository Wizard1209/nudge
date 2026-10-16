interface NudgeAPI {
  platform: "electron";
  save: (data: {
    doing: string;
    bullshit: string;
    nextMinutes: number;
  }) => Promise<void>;
  dismiss: (data: { nextMinutes: number }) => Promise<void>;
  switch: (data: { nextMinutes: number }) => Promise<void>;
  onShow: (callback: (payload?: { gotFocus: boolean }) => void) => () => void;
  /** The configured sprite and its layout, or null when off or invalid. */
  getSprite?: () => Promise<SpriteView | null>;
  /** Fires when a saved sprite change reaches this window. */
  onSprite?: (callback: (sprite: SpriteView | null) => void) => () => void;
}

type NudgeConfig = import("../shared/config").Config;
type SpriteView = import("../shared/sprite").SpriteView;

interface NudgeSettingsAPI {
  getConfig: () => Promise<NudgeConfig>;
  /** Resolves with an error when an enabled sprite fails validation. */
  save: (config: NudgeConfig) => Promise<{ ok: true } | { ok: false; error: string }>;
  /** Native file picker filtered to PNG; null when cancelled. */
  browseSprite: () => Promise<string | null>;
  /** Transactionally toggle OS autostart; resolves with the outcome. */
  toggleAutostart: (
    desired: boolean,
  ) => Promise<{ ok: true } | { ok: false; error: string }>;
  /** Close the Settings window (Cancel / Save done). */
  close: () => void;
  /** Subscribe to the native window's focus loss; returns an unsubscribe. */
  onWindowBlur: (callback: () => void) => () => void;
}

declare global {
  interface Window {
    nudge?: NudgeAPI;
    nudgeSettings?: NudgeSettingsAPI;
  }
}

export {};
