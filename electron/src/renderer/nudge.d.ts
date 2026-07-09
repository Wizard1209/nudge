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
}

interface NudgeConfig {
  hotkey: string;
  default_interval_minutes: number;
  autostart: boolean;
}

interface NudgeSettingsAPI {
  getConfig: () => Promise<NudgeConfig>;
  save: (config: NudgeConfig) => Promise<void>;
  /** Transactionally toggle OS autostart; resolves with the outcome. */
  toggleAutostart: (
    desired: boolean,
  ) => Promise<{ ok: true } | { ok: false; error: string }>;
  /** Close the Settings window (Cancel / Save done). */
  close: () => void;
}

declare global {
  interface Window {
    nudge?: NudgeAPI;
    nudgeSettings?: NudgeSettingsAPI;
  }
}

export {};
