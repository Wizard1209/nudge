interface NudgeAPI {
  platform: "electron";
  /** Whether the OS puts a frosted material under this window's card. */
  backdrop: boolean;
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
type UpdateRelease = import("../shared/updateCheck").Release;

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
  /** The running version, for the Updates row (§9). */
  updateInfo: () => Promise<{ version: string }>;
  /** The update check: GitHub's latest release, or why it could not be read. */
  checkUpdate: () => Promise<
    { ok: true; release: UpdateRelease } | { ok: false; error: string }
  >;
  /** Open a release page of this repository in the browser. */
  openRelease: (url: string) => Promise<void>;
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
