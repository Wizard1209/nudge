export const REPO = "github.com/Wizard1209/nudge";

export const LATEST_URL = "https://api.github.com/repos/Wizard1209/nudge/releases/latest";

export interface Release {
  tag_name: string;
  html_url: string;
  assets: Asset[];
}

export interface Asset {
  name: string;
  digest?: string | null;
  browser_download_url: string;
}

interface Semver {
  core: [number, number, number];
  pre: string[];
}

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

function compare(a: Semver, b: Semver): number {
  for (let i = 0; i < 3; i++) {
    if (a.core[i] !== b.core[i]) return a.core[i] - b.core[i];
  }
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

export function isNewer(tag: string, current: string): boolean {
  const [t, c] = [parseVersion(tag), parseVersion(current)];
  return t !== null && c !== null && compare(t, c) > 0;
}

export function versionOf(tag: string): string {
  return tag.trim().replace(/^v/, "");
}

export function windowsAssetName(tag: string): string {
  return `nudge-v${versionOf(tag)}-x86_64-windows.exe`;
}

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

export function parseDigest(digest: string | null | undefined): string | null {
  const m = /^sha256:([0-9a-fA-F]{64})$/.exec(digest ?? "");
  return m ? m[1].toLowerCase() : null;
}

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
  | { kind: "close" };

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

export const UPDATE_FOOTNOTE = `Checks ${REPO} for new releases, only when pressed.`;

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
