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

/** Absolute path to config.json, alongside the journal in <Documents>/Nudge. */
export function getConfigPath(): string {
  return join(resolveNudgeDir(), "config.json");
}
