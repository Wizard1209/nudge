export interface AutostartProvider {
  enable(): void;
  disable(): void;
  isEnabled(): boolean;
}

export type AutostartError =
  | { kind: "backend"; message: string }
  | { kind: "persist"; cause: Error };

export type ApplyResult = { ok: true } | { ok: false; error: AutostartError };

export function applyAutostart(
  provider: AutostartProvider,
  desired: boolean,
  persist: () => void,
): ApplyResult {
  try {
    if (desired) provider.enable();
    else provider.disable();
  } catch (err) {
    return { ok: false, error: { kind: "backend", message: String(err) } };
  }

  if (provider.isEnabled() !== desired) {
    return {
      ok: false,
      error: {
        kind: "backend",
        message: `system did not confirm autostart=${desired}`,
      },
    };
  }

  try {
    persist();
  } catch (err) {
    return { ok: false, error: { kind: "persist", cause: err as Error } };
  }

  return { ok: true };
}

export interface FakeProviderOptions {
  failEnable?: boolean;
  failDisable?: boolean;
  lieOnConfirm?: boolean;
}

export class FakeProvider implements AutostartProvider {
  private enabled: boolean;
  constructor(
    enabled = false,
    private opts: FakeProviderOptions = {},
  ) {
    this.enabled = enabled;
  }

  enable(): void {
    if (this.opts.failEnable) throw new Error("FakeProvider: enable failed");
    if (!this.opts.lieOnConfirm) this.enabled = true;
  }

  disable(): void {
    if (this.opts.failDisable) throw new Error("FakeProvider: disable failed");
    if (!this.opts.lieOnConfirm) this.enabled = false;
  }

  isEnabled(): boolean {
    return this.enabled;
  }
}
