/**
 * Autostart (launch-at-login) abstraction. The interface is the OS mechanism;
 * `applyAutostart` is the transactional policy that drives it. Both are pure of
 * Electron so the browser Settings sub-app and the unit tests can use a
 * `FakeProvider`, while the native build supplies a real provider.
 */

export interface AutostartProvider {
  /** Register the app to launch at login. Throws on failure. */
  enable(): void;
  /** Unregister. Disabling something already disabled must succeed. */
  disable(): void;
  /** Whether the app is currently registered — used to confirm a change took. */
  isEnabled(): boolean;
}

export type AutostartError =
  | { kind: "backend"; message: string }
  | { kind: "persist"; cause: Error };

export type ApplyResult = { ok: true } | { ok: false; error: AutostartError };

/**
 * Transactionally move autostart to `desired`. Order is load-bearing:
 * OS change → confirm via `isEnabled` → persist. If the OS change throws or
 * isn't confirmed, `persist` is never called, so a failed registry write can't
 * produce a config that claims autostart is on. A persist failure is reported
 * separately (kind "persist") because the OS change already stuck.
 */
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
  /** enable() throws. */
  failEnable?: boolean;
  /** disable() throws. */
  failDisable?: boolean;
  /** enable/disable succeed but isEnabled never reflects the change. */
  lieOnConfirm?: boolean;
}

/** In-memory provider for tests and the browser sub-app. */
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
