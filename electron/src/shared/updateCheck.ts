/**
 * The Settings window's update check (spec §9 "Updates"): the pure half —
 * version order, which asset is the Windows download, what a `digest` says,
 * and the row's state machine. No network, no file system; the shells run
 * the effects this returns.
 */

/** What the row names as the place it asks (§9: the UI says where it looks). */
export const REPO = "github.com/Wizard1209/nudge";

/** The one request the check makes. */
export const LATEST_URL = "https://api.github.com/repos/Wizard1209/nudge/releases/latest";

/** The fields of GitHub's release object the check reads. */
export interface Release {
  tag_name: string;
  html_url: string;
  assets: Asset[];
}

export interface Asset {
  name: string;
  /** `sha256:<hex>`, or absent on a release GitHub has not hashed. */
  digest?: string | null;
  browser_download_url: string;
}

interface Semver {
  core: [number, number, number];
  pre: string[];
}

/** `X.Y.Z`, optionally `v`-prefixed, with semver's `-pre` and `+build`. */
export function parseVersion(text: string): Semver | null {
  const m = /^v?(\d+)\.(\d+)\.(\d+)(?:-([0-9A-Za-z.-]+))?(?:\+[0-9A-Za-z.-]+)?$/.exec(
    text.trim(),
  );
  if (!m) return null;
  return {
    core: [Number(m[1]), Number(m[2]), Number(m[3])],
    pre: m[4] ? m[4].split(".") : [],
  };
}

/** Semver precedence: <0, 0, >0. Build metadata is ignored. */
function compare(a: Semver, b: Semver): number {
  for (let i = 0; i < 3; i++) {
    if (a.core[i] !== b.core[i]) return a.core[i] - b.core[i];
  }
  // A release outranks every pre-release of its own core.
  if (a.pre.length === 0 || b.pre.length === 0) return b.pre.length - a.pre.length;
  for (let i = 0; i < Math.min(a.pre.length, b.pre.length); i++) {
    const [x, y] = [a.pre[i], b.pre[i]];
    const [nx, ny] = [/^\d+$/.test(x), /^\d+$/.test(y)];
    if (nx && ny && Number(x) !== Number(y)) return Number(x) - Number(y);
    if (nx !== ny) return nx ? -1 : 1;
    if (!nx && x !== y) return x < y ? -1 : 1;
  }
  return a.pre.length - b.pre.length;
}

/** Whether `tag` names a version newer than `current`. An older or equal
 * latest — or a tag that is not a version — is not an update. */
export function isNewer(tag: string, current: string): boolean {
  const [t, c] = [parseVersion(tag), parseVersion(current)];
  return t !== null && c !== null && compare(t, c) > 0;
}

/** The version a tag names, as the row shows it: `v0.3.0` → `0.3.0`. */
export function versionOf(tag: string): string {
  return tag.trim().replace(/^v/, "");
}

/** The release's Windows exe, by the name the release workflow gives it. */
export function windowsAssetName(tag: string): string {
  return `nudge-v${versionOf(tag)}-x86_64-windows.exe`;
}

/**
 * The Windows download and the SHA-256 it must hash to, or the reason the
 * update is refused. A missing or unreadable digest refuses: an exe that
 * replaces the running one is installed only when it can be checked.
 */
export function windowsDownload(
  release: Release,
): { ok: true; url: string; sha256: string } | { ok: false; error: string } {
  const name = windowsAssetName(release.tag_name);
  const asset = release.assets.find((a) => a.name === name);
  if (!asset) return { ok: false, error: "The release has no Windows download" };
  const sha256 = parseDigest(asset.digest);
  if (!sha256) return { ok: false, error: "No checksum for the Windows download" };
  return { ok: true, url: asset.browser_download_url, sha256 };
}

/** `sha256:<64 hex>` → the lowercase hex, or null for anything else. */
export function parseDigest(digest: string | null | undefined): string | null {
  const m = /^sha256:([0-9a-fA-F]{64})$/.exec(digest ?? "");
  return m ? m[1].toLowerCase() : null;
}

// --- The row (§9 "Updates") ---

/** What the row's button does once a newer release is found. */
export type UpdateAction = "install" | "openPage";

export type UpdateState =
  | { kind: "idle" }
  | { kind: "checking" }
  | { kind: "upToDate" }
  | { kind: "available"; release: Release }
  | { kind: "installing" }
  | { kind: "restarting" }
  | { kind: "failed"; reason: string };

export type UpdateInput =
  | { kind: "press" }
  | { kind: "checked"; result: { ok: true; release: Release } | { ok: false; error: string } }
  | { kind: "installed"; result: { ok: true } | { ok: false; error: string } };

export type UpdateEffect =
  | { kind: "none" }
  | { kind: "check" }
  | { kind: "install"; release: Release }
  | { kind: "openPage"; url: string }
  /** The new version is in place and the app is restarting: close. */
  | { kind: "close" };

/**
 * One input through the row's table. `press` is the row's one button, whose
 * meaning is the state's: check, or act on the release found. A result that
 * arrives in a state that did not ask for it changes nothing.
 */
export function updateStep(
  state: UpdateState,
  input: UpdateInput,
  current: string,
  action: UpdateAction,
): { state: UpdateState; effect: UpdateEffect } {
  const stay = { state, effect: { kind: "none" } as UpdateEffect };
  switch (input.kind) {
    case "press":
      switch (state.kind) {
        case "idle":
        case "upToDate":
        case "failed":
          return { state: { kind: "checking" }, effect: { kind: "check" } };
        case "available":
          return action === "install"
            ? { state: { kind: "installing" }, effect: { kind: "install", release: state.release } }
            : { state, effect: { kind: "openPage", url: state.release.html_url } };
        default:
          return stay;
      }
    case "checked":
      if (state.kind !== "checking") return stay;
      if (!input.result.ok) return { state: { kind: "failed", reason: input.result.error }, effect: { kind: "none" } };
      return {
        state: isNewer(input.result.release.tag_name, current)
          ? { kind: "available", release: input.result.release }
          : { kind: "upToDate" },
        effect: { kind: "none" },
      };
    case "installed":
      if (state.kind !== "installing") return stay;
      return input.result.ok
        ? { state: { kind: "restarting" }, effect: { kind: "close" } }
        : { state: { kind: "failed", reason: input.result.error }, effect: { kind: "none" } };
  }
}

/** The row's status text, left of its button; empty before the first press. */
export function statusText(state: UpdateState): string {
  switch (state.kind) {
    case "idle":
      return "";
    case "checking":
      return "Checking…";
    case "upToDate":
      return "Up to date";
    case "available":
      return `${versionOf(state.release.tag_name)} available`;
    case "installing":
      return "Updating…";
    case "restarting":
      return "Restarting…";
    case "failed":
      return state.reason;
  }
}

/** The button's label, and whether it can be pressed. */
export function buttonFor(state: UpdateState, action: UpdateAction): { label: string; enabled: boolean } {
  switch (state.kind) {
    case "available":
      return { label: action === "install" ? "Update" : "Open release page", enabled: true };
    case "installing":
    case "restarting":
      return { label: "Update", enabled: false };
    case "checking":
      return { label: "Check for updates", enabled: false };
    default:
      return { label: "Check for updates", enabled: true };
  }
}

/** The footnote under the group: where the check looks, and when. */
export const UPDATE_FOOTNOTE = `Checks ${REPO} for new releases, only when pressed.`;

/** The check's failure, as one short line: no network, or GitHub said no. */
export function describeCheckFailure(
  failure: { kind: "offline" } | { kind: "status"; status: number } | { kind: "unreadable" },
): string {
  switch (failure.kind) {
    case "offline":
      return "Offline: could not reach GitHub";
    case "status":
      return `GitHub refused the request (HTTP ${failure.status})`;
    case "unreadable":
      return "GitHub sent a reply Nudge cannot read";
  }
}
