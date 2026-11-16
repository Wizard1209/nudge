import { app } from "electron";
import { readFileSync, writeFileSync, renameSync, mkdirSync, existsSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";

import { type Config, DEFAULT_CONFIG, normalize } from "../shared/config";
import { resolveNudgeDir } from "./journal";

// Re-export the pure shape + helpers so existing importers (and tests) keep
// using `electron/config` as the entry point, while the browser sub-app can
// import the node-free originals from `shared/config`.
export {
  type Config,
  type ConfigDiff,
  type SpriteConfig,
  DEFAULT_CONFIG,
  normalize,
  diffConfig,
} from "../shared/config";

export interface LoadResult {
  config: Config;
  error: Error | null;
}

export function loadConfig(path: string): LoadResult {
  if (!existsSync(path)) {
    return { config: { ...DEFAULT_CONFIG }, error: null };
  }
  let parsed: unknown;
  try {
    parsed = JSON.parse(readFileSync(path, "utf-8"));
  } catch (err) {
    return { config: { ...DEFAULT_CONFIG }, error: err as Error };
  }
  return { config: normalize(parsed), error: null };
}

export function saveConfig(path: string, config: Config): void {
  const dir = dirname(path);
  if (!existsSync(dir)) mkdirSync(dir, { recursive: true });
  const tmp = path + ".tmp";
  writeFileSync(tmp, JSON.stringify(config, null, 2) + "\n", "utf-8");
  renameSync(tmp, path);
}

/**
 * Directory holding config.json: `<appData>/Nudge`, the OS's per-user
 * app-settings folder (`~/Library/Application Support/Nudge` on macOS,
 * `%APPDATA%\Nudge` on Windows). Not `<Documents>/Nudge` beside the journal:
 * the settings belong to one machine, and Documents is synced.
 */
export function configDirFor(appData: string): string {
  return join(appData, "Nudge");
}

/**
 * Move a config left in `<Documents>/Nudge` to `destination`, once, and
 * return the file to run against. If the destination exists, or there is
 * nothing to move, nothing is touched.
 *
 * A rename is tried first. Where it is refused — Documents on another drive
 * or a network share — the file is copied next to the destination, checked
 * byte for byte, renamed into place, and only then is the original removed.
 * If any step fails, nothing is left at the destination and the legacy path
 * is returned: this session runs on the old file and the next start tries
 * again. `rename` is a parameter so a refused rename can be tested.
 */
export function migrateLegacyConfig(
  legacy: string,
  destination: string,
  rename: (from: string, to: string) => void = renameSync,
): string {
  if (legacy === destination || existsSync(destination) || !existsSync(legacy)) {
    return destination;
  }
  try {
    mkdirSync(dirname(destination), { recursive: true });
    try {
      rename(legacy, destination);
    } catch {
      moveByCopy(legacy, destination, rename);
    }
    console.error(`[nudge] moved config ${legacy} -> ${destination}`);
    return destination;
  } catch (err) {
    console.error(
      `[nudge] could not move config ${legacy} -> ${destination}: ${err}; using it where it is`,
    );
    return legacy;
  }
}

/**
 * Copy, verify, rename into place, then remove the original. If the original
 * cannot be removed, the destination is already the live file (the next start
 * finds it and leaves the original alone), so that is logged, not undone.
 */
function moveByCopy(
  legacy: string,
  destination: string,
  rename: (from: string, to: string) => void,
): void {
  const bytes = readFileSync(legacy);
  const tmp = destination + ".tmp";
  try {
    writeFileSync(tmp, bytes);
    if (!readFileSync(tmp).equals(bytes)) throw new Error("the copy differs from the original");
    rename(tmp, destination);
  } catch (err) {
    try {
      rmSync(tmp, { force: true });
    } catch {
      // Whatever occupies the temp path is not ours to remove.
    }
    throw err;
  }
  try {
    rmSync(legacy);
  } catch (err) {
    console.error(
      `[nudge] config copied to ${destination}, but ${legacy} could not be removed (${err}); it is no longer read`,
    );
  }
}

let resolvedPath: string | null = null;

/**
 * Absolute path to config.json (see {@link configDirFor}). On first call,
 * moves a config left in `<Documents>/Nudge` to the new location, so the move
 * happens before anything reads the file. If the move fails, this is the
 * legacy path for the rest of the session.
 */
export function getConfigPath(): string {
  if (resolvedPath) return resolvedPath;
  const nudgeDir = resolveNudgeDir();
  // NUDGE_DIR is the test override: everything stays inside it.
  const dir = process.env.NUDGE_DIR
    ? nudgeDir
    : configDirFor(app.getPath("appData"));
  resolvedPath = migrateLegacyConfig(join(nudgeDir, "config.json"), join(dir, "config.json"));
  return resolvedPath;
}
