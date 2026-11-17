interface NudgeAPI {
  platform: "electron";
  backdrop: boolean;
  save: (data: {
    doing: string;
    bullshit: string;
    nextMinutes: number;
  }) => Promise<void>;
  dismiss: (data: { nextMinutes: number }) => Promise<void>;
  switch: (data: { nextMinutes: number }) => Promise<void>;
  onShow: (callback: (payload?: { gotFocus: boolean }) => void) => () => void;
  getSprite?: () => Promise<SpriteView | null>;
  onSprite?: (callback: (sprite: SpriteView | null) => void) => () => void;
}

type NudgeConfig = import("../shared/config").Config;
type SpriteView = import("../shared/sprite").SpriteView;
type UpdateRelease = import("../shared/updateCheck").Release;

interface NudgeSettingsAPI {
  getConfig: () => Promise<NudgeConfig>;
  save: (config: NudgeConfig) => Promise<{ ok: true } | { ok: false; error: string }>;
  browseSprite: () => Promise<string | null>;
  toggleAutostart: (
    desired: boolean,
  ) => Promise<{ ok: true } | { ok: false; error: string }>;
  close: () => void;
  updateInfo: () => Promise<{ version: string }>;
  checkUpdate: () => Promise<
    { ok: true; release: UpdateRelease } | { ok: false; error: string }
  >;
  openRelease: (url: string) => Promise<void>;
  onWindowBlur: (callback: () => void) => () => void;
}

declare global {
  interface Window {
    nudge?: NudgeAPI;
    nudgeSettings?: NudgeSettingsAPI;
  }
}

export {};
