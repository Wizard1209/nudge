import { app } from "electron";
import { readFileSync, writeFileSync, renameSync, mkdirSync, existsSync } from "node:fs";
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
 * Move a config left in the old location to the new one, once. Never copies;
 * if the destination exists, or there is nothing to move, nothing is touched.
 */
export function migrateLegacyConfig(legacy: string, destination: string): void {
  if (legacy === destination || existsSync(destination) || !existsSync(legacy)) return;
  try {
    mkdirSync(dirname(destination), { recursive: true });
    renameSync(legacy, destination);
    console.error(`[nudge] moved config ${legacy} -> ${destination}`);
  } catch (err) {
    console.error(`[nudge] could not move config ${legacy} -> ${destination}: ${err}`);
  }
}

let resolvedPath: string | null = null;

/**
 * Absolute path to config.json (see {@link configDirFor}). On first call,
 * moves a config left in `<Documents>/Nudge` to the new location, so the move
 * happens before anything reads the file.
 */
export function getConfigPath(): string {
  if (resolvedPath) return resolvedPath;
  const nudgeDir = resolveNudgeDir();
  // NUDGE_DIR is the test override: everything stays inside it.
  const dir = process.env.NUDGE_DIR
    ? nudgeDir
    : configDirFor(app.getPath("appData"));
  resolvedPath = join(dir, "config.json");
  if (dir !== nudgeDir) migrateLegacyConfig(join(nudgeDir, "config.json"), resolvedPath);
  return resolvedPath;
}
