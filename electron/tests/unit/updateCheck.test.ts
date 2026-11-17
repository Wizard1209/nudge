import { describe, it, expect } from "vitest";
import {
  isNewer,
  windowsDownload,
  parseDigest,
  updateStep,
  statusText,
  buttonFor,
  describeCheckFailure,
  UPDATE_FOOTNOTE,
  type Release,
  type UpdateState,
  type UpdateInput,
  type UpdateAction,
} from "../../src/shared/updateCheck";

const HEX = "b48b42654c96ae880e1fec6786ab7ad67f63a9dd44d99b2b815b84c10239694e";

const release = (tag: string, assets: Release["assets"] = []): Release => ({
  tag_name: tag,
  html_url: `https://github.com/Wizard1209/nudge/releases/tag/${tag}`,
  assets,
});

const exe = (tag: string, digest: string | null = `sha256:${HEX}`) => ({
  name: `nudge-${tag}-x86_64-windows.exe`,
  digest,
  browser_download_url: `https://example.invalid/${tag}.exe`,
});

describe("version order (§9: a newer release only)", () => {
  it("a higher tag, v-prefixed or not, is newer", () => {
    expect(isNewer("v0.3.0", "0.2.2")).toBe(true);
    expect(isNewer("0.2.3", "0.2.2")).toBe(true);
    expect(isNewer("v1.0.0", "0.99.99")).toBe(true);
    expect(isNewer("v0.10.0", "0.9.0")).toBe(true);
  });

  it("the same or an older latest is up to date", () => {
    expect(isNewer("v0.2.2", "0.2.2")).toBe(false);
    expect(isNewer("v0.2.1", "0.2.2")).toBe(false);
  });

  it("a release outranks a pre-release of its own version", () => {
    expect(isNewer("v0.3.0", "0.3.0-dev")).toBe(true);
    expect(isNewer("v0.3.0-rc.1", "0.3.0")).toBe(false);
    expect(isNewer("v0.3.0-rc.2", "0.3.0-rc.1")).toBe(true);
  });

  it("a tag that is not a version is never an update", () => {
    expect(isNewer("nightly", "0.2.2")).toBe(false);
    expect(isNewer("v0.3", "0.2.2")).toBe(false);
  });
});

describe("the Windows download", () => {
  it("is the asset named for the tag, with its digest", () => {
    const r = release("v0.3.0", [
      { ...exe("v0.3.0"), name: "nudge-v0.3.0-macos-universal.dmg" },
      exe("v0.3.0"),
    ]);
    expect(windowsDownload(r)).toEqual({
      ok: true,
      url: "https://example.invalid/v0.3.0.exe",
      sha256: HEX,
    });
  });

  it("is refused when the release has none", () => {
    expect(windowsDownload(release("v0.3.0", [exe("v0.2.9")]))).toEqual({
      ok: false,
      error: "The release has no Windows download",
    });
  });

  it("is refused when the digest is missing or is not a sha256", () => {
    for (const digest of [null, "", "sha512:abc", `sha256:${HEX.slice(1)}`]) {
      expect(windowsDownload(release("v0.3.0", [exe("v0.3.0", digest)]))).toEqual({
        ok: false,
        error: "No checksum for the Windows download",
      });
    }
  });

  it("reads a digest case-insensitively", () => {
    expect(parseDigest(`sha256:${HEX.toUpperCase()}`)).toBe(HEX);
  });
});

describe("the Updates row's table (§9)", () => {
  const run = (inputs: UpdateInput[], action: UpdateAction = "install", current = "0.2.2") => {
    let state: UpdateState = { kind: "idle" };
    const effects = [];
    for (const input of inputs) {
      const out = updateStep(state, input, current, action);
      state = out.state;
      effects.push(out.effect.kind);
    }
    return { state, effects };
  };
  const found = (tag: string): UpdateInput => ({
    kind: "checked",
    result: { ok: true, release: release(tag) },
  });

  it("nothing is asked before the button is pressed", () => {
    expect(statusText({ kind: "idle" })).toBe("");
    expect(buttonFor({ kind: "idle" }, "install")).toEqual({ label: "Check for updates", enabled: true });
    expect(UPDATE_FOOTNOTE).toBe("Checks github.com/Wizard1209/nudge for new releases, only when pressed.");
  });

  it("press checks; the button is greyed while checking", () => {
    const { state, effects } = run([{ kind: "press" }]);
    expect(state.kind).toBe("checking");
    expect(effects).toEqual(["check"]);
    expect(statusText(state)).toBe("Checking…");
    expect(buttonFor(state, "install").enabled).toBe(false);
  });

  it("a newer release offers the platform's action", () => {
    const { state } = run([{ kind: "press" }, found("v0.3.0")]);
    expect(statusText(state)).toBe("0.3.0 available");
    expect(buttonFor(state, "install").label).toBe("Update");
    expect(buttonFor(state, "openPage").label).toBe("Open release page");
  });

  it("the same or an older release is up to date, and can be checked again", () => {
    for (const tag of ["v0.2.2", "v0.2.1"]) {
      const { state } = run([{ kind: "press" }, found(tag)]);
      expect(statusText(state)).toBe("Up to date");
      expect(buttonFor(state, "install")).toEqual({ label: "Check for updates", enabled: true });
    }
  });

  it("a failed check shows its reason and can be retried", () => {
    const { state, effects } = run([
      { kind: "press" },
      { kind: "checked", result: { ok: false, error: describeCheckFailure({ kind: "offline" }) } },
      { kind: "press" },
    ]);
    expect(effects).toEqual(["check", "none", "check"]);
    expect(state.kind).toBe("checking");
    expect(describeCheckFailure({ kind: "status", status: 403 })).toBe(
      "GitHub refused the request (HTTP 403)",
    );
  });

  it("Update installs, then the window closes for the restart", () => {
    const { state, effects } = run([
      { kind: "press" },
      found("v0.3.0"),
      { kind: "press" },
      { kind: "installed", result: { ok: true } },
    ]);
    expect(effects).toEqual(["check", "none", "install", "close"]);
    expect(state.kind).toBe("restarting");
  });

  it("a refused install shows its reason and offers a fresh check", () => {
    const { state } = run([
      { kind: "press" },
      found("v0.3.0"),
      { kind: "press" },
      { kind: "installed", result: { ok: false, error: "Download does not match its checksum" } },
    ]);
    expect(statusText(state)).toBe("Download does not match its checksum");
    expect(buttonFor(state, "install").label).toBe("Check for updates");
  });

  it("Open release page opens it and stays", () => {
    const { state, effects } = run([{ kind: "press" }, found("v0.3.0"), { kind: "press" }], "openPage");
    expect(effects).toEqual(["check", "none", "openPage"]);
    expect(state.kind).toBe("available");
  });

  it("a result nobody asked for changes nothing", () => {
    expect(run([found("v0.3.0")]).state.kind).toBe("idle");
    expect(run([{ kind: "installed", result: { ok: true } }]).effects).toEqual(["none"]);
  });
});
