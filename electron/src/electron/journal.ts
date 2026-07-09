import { appendFileSync, existsSync, mkdirSync, readFileSync } from "node:fs";
import { execSync } from "node:child_process";
import { dirname, join } from "node:path";
import { app } from "electron";

export interface JournalEvent {
  schema_version: 1;
  event_type: "submitted";
  entry_id: string;
  captured_at: string;
  implementation: "electron";
  trigger_source: "timer" | "manual";
  doing: string;
  bullshit: string;
  next_interval_minutes: number;
  prompt_version?: string;
  input_method?: "keyboard" | "voice" | string;
  metadata?: Record<string, unknown>;
}

export class JournalValidationError extends Error {
  constructor(
    public field: string,
    public reason: string,
  ) {
    super(`Invalid journal event: ${field} — ${reason}`);
    this.name = "JournalValidationError";
  }
}

const TIMESTAMP_RE =
  /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}(Z|[+-]\d{2}:\d{2})$/;

export function formatTimestamp(d: Date = new Date()): string {
  // toISOString() produces "YYYY-MM-DDTHH:mm:ss.sssZ" — Z is a valid RFC 3339 offset.
  return d.toISOString();
}

// Spec §4: Enter writes the journal only if at least one of doing/bullshit
// has real content. Whitespace-only counts as empty so a stray Enter on an
// untouched form doesn't leave a blank row in the journal. This also enables
// the "change interval without journaling" workflow.
export function isJournalable(doing: string, bullshit: string): boolean {
  return doing.trim() !== "" || bullshit.trim() !== "";
}

function validate(ev: JournalEvent): void {
  if (ev.schema_version !== 1)
    throw new JournalValidationError("schema_version", "must be 1");
  if (ev.event_type !== "submitted")
    throw new JournalValidationError("event_type", 'must be "submitted" in v1');
  if (!ev.entry_id)
    throw new JournalValidationError("entry_id", "must be non-empty");
  if (!TIMESTAMP_RE.test(ev.captured_at))
    throw new JournalValidationError(
      "captured_at",
      "must be RFC 3339 with ms and offset",
    );
  if (!ev.implementation)
    throw new JournalValidationError("implementation", "must be non-empty");
  if (ev.trigger_source !== "timer" && ev.trigger_source !== "manual")
    throw new JournalValidationError(
      "trigger_source",
      'must be "timer" or "manual"',
    );
  if (typeof ev.doing !== "string")
    throw new JournalValidationError("doing", "must be string");
  if (typeof ev.bullshit !== "string")
    throw new JournalValidationError("bullshit", "must be string");
  if (
    !Number.isFinite(ev.next_interval_minutes) ||
    ev.next_interval_minutes <= 0
  )
    throw new JournalValidationError(
      "next_interval_minutes",
      "must be finite and > 0",
    );
}

// --- Path resolution ---

let cachedDir: string | null = null;

function isWSL(): boolean {
  if (process.platform !== "linux") return false;
  if (process.env.WSL_DISTRO_NAME) return true;
  try {
    return readFileSync("/proc/version", "utf-8")
      .toLowerCase()
      .includes("microsoft");
  } catch {
    return false;
  }
}

/**
 * Resolve the shared `<Documents>/Nudge` directory that holds both the journal
 * and config.json. On WSL this bridges to the Windows user profile via
 * cmd.exe + wslpath; elsewhere it uses Electron's documents path. Cached.
 */
export function resolveNudgeDir(): string {
  if (cachedDir) return cachedDir;

  try {
    if (isWSL()) {
      // cwd=/mnt/c avoids cmd.exe's "UNC paths not supported" warning
      // when started from a \\wsl.localhost\... directory.
      const raw = execSync('cmd.exe /C "echo %USERPROFILE%"', {
        encoding: "utf-8",
        stdio: ["ignore", "pipe", "ignore"],
        cwd: "/mnt/c",
      });
      const userProfile = raw.trim().split(/\r?\n/).pop()!.trim();
      const winPath = userProfile + "\\Documents\\Nudge";
      cachedDir = execSync(`wslpath -u '${winPath}'`, {
        encoding: "utf-8",
      }).trim();
    } else {
      cachedDir = join(app.getPath("documents"), "Nudge");
    }
  } catch {
    cachedDir = join(app.getPath("documents"), "Nudge");
  }

  return cachedDir;
}

export function getJournalPath(): string {
  return join(resolveNudgeDir(), "journal-electron.ndjson");
}

// --- Write ---

export function writeEvent(ev: JournalEvent, filePath?: string): void {
  validate(ev);
  filePath = filePath ?? getJournalPath();
  const dir = dirname(filePath);
  if (!existsSync(dir)) mkdirSync(dir, { recursive: true });
  appendFileSync(filePath, JSON.stringify(ev) + "\n", "utf-8");
}
