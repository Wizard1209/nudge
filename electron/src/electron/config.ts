import { app } from "electron";
import { readFileSync, writeFileSync, renameSync, mkdirSync, existsSync, rmSync } from "node:fs";
import { dirname, join } from "node:path";

import { type Config, DEFAULT_CONFIG, normalize } from "../shared/config";
import { resolveNudgeDir } from "./journal";

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

export function configDirFor(appData: string): string {
  return join(appData, "Nudge");
}

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

export function getConfigPath(): string {
  if (resolvedPath) return resolvedPath;
  const nudgeDir = resolveNudgeDir();
  const dir = process.env.NUDGE_DIR
    ? nudgeDir
    : configDirFor(app.getPath("appData"));
  resolvedPath = migrateLegacyConfig(join(nudgeDir, "config.json"), join(dir, "config.json"));
  return resolvedPath;
}
