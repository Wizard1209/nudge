# Journal Specification

Canonical journaling contract for Nudge. Both the Electron and Rust implementations
must follow this contract so their journal data is mutually writable, readable,
mergeable, and analysable.

This spec defines three layers:

1. The logical journal model
2. The NDJSON storage binding for v1
3. The operational rules for reading, writing, recovery, and testing

## Goals

- Preserve the meaning of journal data across implementations
- Keep append-only writes simple and robust
- Make future schema evolution explicit
- Support richer features without changing the storage model again
- Keep analysis and export straightforward

## Non-goals

- End-user encryption at rest
- Multi-device sync
- Concurrent writes to a single shared file in v1
- Full transactional guarantees comparable to a database

## Canonical journal model

The canonical unit is a `JournalEvent`.

In v1, Nudge persists only `submitted` events. The model is intentionally broader
than the initial stored subset so future features do not require a storage redesign.

### Event shape

| Field | Type | Required | Meaning |
|---|---|---|---|
| `schema_version` | integer | yes | Logical schema version. For this spec: `1` |
| `event_type` | string | yes | Event kind. In v1 persisted values: `submitted` |
| `entry_id` | string | yes | Stable unique identifier for deduplication and merge |
| `captured_at` | string | yes | RFC 3339 timestamp with UTC offset and milliseconds |
| `implementation` | string | yes | Producer implementation, e.g. `rust`, `electron` |
| `trigger_source` | string | yes | Why the popup appeared: `timer` or `manual` |
| `doing` | string | yes | Free-form answer to "What am I doing?" |
| `bullshit` | string | yes | Free-form answer to the reflection prompt |
| `next_interval_minutes` | number | yes | Requested next interval in minutes |
| `prompt_version` | string | no | Identifier of prompt wording/layout version |
| `input_method` | string | no | How the entry was produced, e.g. `keyboard`, `voice` |
| `metadata` | object | no | Forward-compatible extension bucket |

### Current persisted subset

Every line written in v1 must contain at least:

- `schema_version`
- `event_type`
- `entry_id`
- `captured_at`
- `implementation`
- `trigger_source`
- `doing`
- `bullshit`
- `next_interval_minutes`

### Invariants

- `schema_version` must equal `1`
- `event_type` must equal `submitted` in v1 persisted data
- `entry_id` must be globally unique within a journal file and should be globally
  unique across implementations
- `captured_at` is the time the submission was accepted by the app, not the time
  the popup was first shown
- `doing` and `bullshit` may be empty strings unless the product later tightens UI validation
- `next_interval_minutes` must be a positive decimal number greater than `0`
- Unknown top-level keys must be tolerated by readers

### Reserved future event types

These are not written in v1, but the model reserves them:

- `prompt_shown`
- `dismissed`
- `write_failed`
- `edited`

## File location

Journal lives in **Windows Documents**, in a dedicated `Nudge/` subfolder:

```text
%USERPROFILE%\Documents\Nudge\journal-<impl>.ndjson
```

Per-implementation filenames avoid concurrent-write hazards:

| Implementation | File |
|---|---|
| Electron | `journal-electron.ndjson` |
| Rust | `journal-rust.ndjson` |

Parent directory must be created recursively if missing.

### Path resolution by host

| Host | How to resolve |
|---|---|
| Native Windows | Win32 `SHGetKnownFolderPath(FOLDERID_Documents)` or equivalent |
| WSL (Linux app -> Windows FS) | `cmd.exe /C "echo %USERPROFILE%"` + append `\Documents\Nudge` + `wslpath -u` |
| Linux/macOS (no Windows) | Fall back to platform Documents folder (`$XDG_DOCUMENTS_DIR` / `~/Documents`) |

WSL caveat: run `cmd.exe` with `cwd` set to a Windows-accessible path such as `/mnt/c`
to avoid the "UNC paths are not supported" warning polluting stdout.

For the Rust implementation targeting native Windows, only the first row applies.

## NDJSON binding

### Format

- Encoding: UTF-8
- BOM: do not write BOM; tolerate BOM on the first line when reading
- Line ending: write `\n`; tolerate `\r\n` on existing files
- Storage shape: one JSON object per line
- File extension: `.ndjson`

### Example

```jsonl
{"schema_version":1,"event_type":"submitted","entry_id":"01JS1S8R5W4Y4S4M8Q6A8X7R2V","captured_at":"2026-04-17T14:30:00.000+02:00","implementation":"rust","trigger_source":"timer","doing":"writing requirements","bullshit":"no","next_interval_minutes":10}
{"schema_version":1,"event_type":"submitted","entry_id":"01JS1S9FDRW4K4M7R4F5R9A5A2","captured_at":"2026-04-17T14:40:00.000+02:00","implementation":"electron","trigger_source":"manual","doing":"watching YouTube","bullshit":"yes","next_interval_minutes":5}
```

### Canonical field encoding

- `schema_version`: JSON integer
- `event_type`: lowercase ASCII token
- `entry_id`: opaque string; ULID is recommended
- `captured_at`: RFC 3339 string with exactly 3 fractional second digits and explicit offset
- `implementation`: lowercase ASCII token such as `rust` or `electron`
- `trigger_source`: lowercase ASCII token, currently `timer` or `manual`
- `doing`: JSON string, preserve user text exactly
- `bullshit`: JSON string, preserve user text exactly
- `next_interval_minutes`: JSON number; integer or decimal, no string wrapper
- `metadata`: JSON object if present

### Canonical timestamp examples

- `2026-04-17T14:30:00.000+02:00`
- `2026-04-17T12:30:00.125Z`

Implementations must not write timestamps without an offset.

## Writer contract

Given a validated `JournalEvent` and resolved `filePath`:

```text
1. ensure parent directory exists
2. serialize the event as a single-line JSON object
3. append "\n"
4. append-write the bytes to filePath
5. if the write fails, report a typed error to the caller
```

### Writer invariants

- The file is append-only
- Each logical event occupies exactly one physical line
- Writers must never rewrite, reorder, or compact existing entries
- Writers must never emit pretty-printed or multi-line JSON
- Writers must validate required fields before writing
- Writers must not silently coerce invalid values except where the product spec
  explicitly allows normalization

### Validation rules before write

- `schema_version` must be `1`
- `event_type` must be `submitted`
- `entry_id` must be non-empty
- `captured_at` must include offset and milliseconds
- `implementation` must be non-empty
- `trigger_source` must be one of `timer` or `manual`
- `next_interval_minutes` must be finite and greater than `0`
- `doing` and `bullshit` must be valid UTF-8 strings

### Durability

V1 requires successful append to the host file descriptor or handle before the UI
may treat the submission as persisted.

V1 does not require `fsync` or equivalent after every entry. If stronger crash
durability is needed later, that must be a separate explicitly versioned policy
decision, not an implementation accident.

## Reader contract

Readers must process the file line by line.

For each non-empty line:

1. strip a BOM only if it appears at the beginning of the first line
2. strip a trailing `\r`
3. parse the line as a single JSON object
4. validate required fields for the declared `schema_version`
5. surface or record any invalid line as a structured read error

### Reader tolerance rules

Readers must tolerate:

- UTF-8 BOM on the first line
- `\r\n` line endings
- Unknown top-level keys
- Optional missing fields that are not required by this version

Readers must reject the line as invalid if:

- it is not valid JSON
- it is not a JSON object
- required fields are missing
- `schema_version` is unknown and the reader has no migration support
- `captured_at` has no UTC offset
- `next_interval_minutes` is missing, non-numeric, non-finite, or `<= 0`

### Corruption handling

The spec distinguishes file-level access from line-level validity.

- If the file cannot be opened, this is a file access error
- If a line is malformed, this is a data error for that line
- Readers may continue past malformed lines for analysis tools
- Interactive product code should surface malformed-line errors clearly

If the last line is truncated, readers may treat only that final line as invalid
and still recover earlier valid entries.

## UI error-handling contract

On any write error:

- keep the popup visible
- preserve all typed user data
- do not reset the countdown
- show a user-visible error that includes the path and OS or validation detail

This behavior matters more than the exact UI wording.

## Schema evolution

The logical schema is versioned independently of implementation.

### Rules

- New optional fields may be added in schema version `1`
- Existing field meanings must not change silently
- Removing or renaming a required field requires a new `schema_version`
- Changing timestamp format requires a new `schema_version`
- Changing storage type requires a new storage-binding spec, but does not
  necessarily require a new logical schema version

### Compatibility posture

- Writers must write only fields they understand correctly
- Readers should ignore unknown fields
- Readers must reject unknown required semantics for an unsupported schema version

## Usage scenarios this spec is intended to support

- Timer-triggered submission
- Manual-popup submission
- Empty-text submission if the UI allows it
- Unicode text, punctuation, quotes, commas, and embedded newlines
- Voice input or other non-keyboard capture methods
- Future LLM-derived annotations stored under `metadata`
- Multiple implementations writing separate files that are later merged
- Best-effort recovery from a malformed final line
- Historical analysis across old and new clients

## Merge and analysis guidance

Per-implementation files are an operational compromise, not separate logical journals.

For analysis, consumers may concatenate all implementation files and sort by:

1. `captured_at`
2. `entry_id`

If deduplication is needed, use `entry_id` as the primary key.

## Privacy and operational notes

- Journal content is sensitive user text
- Storage is plaintext NDJSON in the user's Documents folder
- The spec intentionally favors local inspectability and portability over secrecy
- If encryption, remote sync, or secure deletion become requirements, those need
  separate product and storage specs

## Test checklist

Minimum test coverage every implementation must pass. Writer functions should accept
an explicit `filePath` so tests can target a temp directory.

1. Fresh write creates parent directories and one valid NDJSON line
2. Two consecutive writes append two lines in order
3. Unicode text round-trips unchanged
4. Quotes, commas, and newlines inside strings round-trip unchanged
5. Existing file with BOM on first line is accepted by readers
6. Existing file with CRLF is accepted by readers
7. Truncated final line does not invalidate earlier valid lines
8. Invalid JSON line is reported as a structured read error
9. Missing required field is reported as a structured read error
10. Unknown extra field is tolerated by readers
11. Unknown `schema_version` is rejected unless explicitly supported
12. Write failure keeps UI state intact and does not reset the timer
13. `next_interval_minutes <= 0` is rejected before write unless the product layer
    normalizes it before constructing the event
14. Cross-implementation golden files are readable by both implementations

## Open implementation notes

This spec does not require both implementations to share one live file.

If a future version requires concurrent writes to a shared file, that needs:

- explicit locking semantics
- a crash-safety policy
- a revised operational section

At that point, SQLite may be a better binding than NDJSON.
