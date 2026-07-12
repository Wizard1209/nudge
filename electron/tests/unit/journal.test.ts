import { describe, it, expect, beforeEach, afterEach } from "vitest";
import {
  mkdtempSync,
  rmSync,
  readFileSync,
  existsSync,
  statSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import {
  writeEvent,
  formatTimestamp,
  isJournalable,
  JournalValidationError,
  type JournalEvent,
} from "../../src/electron/journal";

const EVENT: JournalEvent = {
  schema_version: 1,
  event_type: "submitted",
  entry_id: "01JS1S8R5W4Y4S4M8Q6A8X7R2V",
  captured_at: "2026-04-17T10:00:00.000Z",
  implementation: "electron",
  trigger_source: "timer",
  doing: "writing tests",
  bullshit: "no",
  next_interval_minutes: 10,
};

describe("journal / writeEvent", () => {
  let tmp: string;
  let file: string;

  beforeEach(() => {
    tmp = mkdtempSync(join(tmpdir(), "nudge-journal-"));
    file = join(tmp, "journal.ndjson");
  });

  afterEach(() => {
    rmSync(tmp, { recursive: true, force: true });
  });

  it("creates parent directories and writes exactly one NDJSON line on first write", () => {
    const nested = join(tmp, "deep", "nested", "dir", "journal.ndjson");
    writeEvent(EVENT, nested);

    expect(existsSync(nested)).toBe(true);
    const content = readFileSync(nested, "utf-8");
    const lines = content.split("\n");
    expect(lines).toHaveLength(2);
    expect(lines[1]).toBe("");
    expect(JSON.parse(lines[0])).toEqual(EVENT);
  });

  it("appends two consecutive writes in order without altering the first line", () => {
    writeEvent(EVENT, file);
    const firstSnapshot = readFileSync(file, "utf-8");

    writeEvent(
      { ...EVENT, entry_id: "01JS1S9FDRW4K4M7R4F5R9A5A2", doing: "second" },
      file,
    );

    const content = readFileSync(file, "utf-8");
    expect(content.startsWith(firstSnapshot)).toBe(true);

    const lines = content.split("\n").filter(Boolean);
    expect(lines).toHaveLength(2);
    expect(JSON.parse(lines[0]).doing).toBe("writing tests");
    expect(JSON.parse(lines[1]).doing).toBe("second");
  });

  it("serializes each event as a single-line JSON object (no pretty-print)", () => {
    writeEvent(EVENT, file);
    const content = readFileSync(file, "utf-8");
    expect(content.match(/\n/g)).toHaveLength(1);
    expect(content.endsWith("\n")).toBe(true);
    expect(content).not.toContain("  ");
    expect(content).not.toContain("{\n");
  });

  it("round-trips Unicode text byte-for-byte", () => {
    const ev = { ...EVENT, doing: "пишу тесты 🧪", bullshit: "нет, работа" };
    writeEvent(ev, file);
    const line = readFileSync(file, "utf-8").trimEnd();
    const parsed = JSON.parse(line);
    expect(parsed.doing).toBe("пишу тесты 🧪");
    expect(parsed.bullshit).toBe("нет, работа");
  });

  it("round-trips strings containing quotes, commas, and embedded newlines", () => {
    const ev = {
      ...EVENT,
      doing: 'tea, coffee, or "water"',
      bullshit: "line1\nline2",
    };
    writeEvent(ev, file);
    const line = readFileSync(file, "utf-8").trimEnd();
    expect(line).not.toContain("\n");
    const parsed = JSON.parse(line);
    expect(parsed.doing).toBe('tea, coffee, or "water"');
    expect(parsed.bullshit).toBe("line1\nline2");
  });

  it("rejects next_interval_minutes <= 0 and writes nothing", () => {
    expect(() =>
      writeEvent({ ...EVENT, next_interval_minutes: 0 }, file),
    ).toThrow(JournalValidationError);
    expect(existsSync(file)).toBe(false);

    expect(() =>
      writeEvent({ ...EVENT, next_interval_minutes: -1 }, file),
    ).toThrow(JournalValidationError);
    expect(existsSync(file)).toBe(false);
  });

  it("rejects an empty entry_id and writes nothing", () => {
    expect(() => writeEvent({ ...EVENT, entry_id: "" }, file)).toThrow(
      JournalValidationError,
    );
    expect(existsSync(file)).toBe(false);
  });

  it("rejects an invalid trigger_source and writes nothing", () => {
    expect(() =>
      // @ts-expect-error intentional invalid value
      writeEvent({ ...EVENT, trigger_source: "cron" }, file),
    ).toThrow(JournalValidationError);
    expect(existsSync(file)).toBe(false);
  });

  it("rejects a captured_at without milliseconds or offset and writes nothing", () => {
    expect(() =>
      writeEvent({ ...EVENT, captured_at: "2026-04-17T10:00:00Z" }, file),
    ).toThrow(JournalValidationError);
    expect(existsSync(file)).toBe(false);
  });

  it("does not modify file size when validation fails on a pre-existing file", () => {
    writeEvent(EVENT, file);
    const sizeBefore = statSync(file).size;

    expect(() =>
      writeEvent({ ...EVENT, next_interval_minutes: 0 }, file),
    ).toThrow(JournalValidationError);

    expect(statSync(file).size).toBe(sizeBefore);
  });
});

describe("journal / formatTimestamp", () => {
  it("matches RFC 3339 with millisecond precision and UTC offset", () => {
    const out = formatTimestamp(new Date(Date.UTC(2026, 3, 17, 10, 0, 0, 7)));
    expect(out).toMatch(
      /^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\.\d{3}(Z|[+-]\d{2}:\d{2})$/,
    );
  });

  it("round-trips through Date preserving millisecond precision", () => {
    const d = new Date(Date.UTC(2026, 3, 17, 10, 0, 0, 7));
    const parsed = new Date(formatTimestamp(d));
    expect(parsed.getTime()).toBe(d.getTime());
  });
});

describe("journal / isJournalable", () => {
  // Spec §4: "If at least one of doing / bullshit is non-empty — writes a
  // journal entry." Used to support the "change interval without journaling"
  // workflow: manual open → tweak minutes → Enter with empty fields → timer
  // is updated, journal stays untouched.
  it("returns true when doing has content", () => {
    expect(isJournalable("writing tests", "")).toBe(true);
  });

  it("returns true when bullshit has content", () => {
    expect(isJournalable("", "yes")).toBe(true);
  });

  it("returns true when both have content", () => {
    expect(isJournalable("writing tests", "no")).toBe(true);
  });

  it("returns false when both are empty", () => {
    expect(isJournalable("", "")).toBe(false);
  });

  it("returns false when both are whitespace only", () => {
    // Whitespace-only is treated as empty — typing spaces by accident or
    // bumping Enter from a focused empty field should not pollute the journal.
    expect(isJournalable("   ", "\t \n")).toBe(false);
  });
});
