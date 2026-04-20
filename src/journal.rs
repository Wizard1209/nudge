use serde::{Deserialize, Serialize};
use std::fmt;

/// A single journal event in NDJSON format per the journal spec.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JournalEvent {
    pub schema_version: u32,
    pub event_type: String,
    pub entry_id: String,
    pub captured_at: String,
    pub implementation: String,
    pub trigger_source: String,
    pub doing: String,
    pub bullshit: String,
    pub next_interval_minutes: f64,
    // TODO: optional fields (prompt_version, input_method, metadata)
}

#[derive(Debug, Clone)]
pub enum JournalError {
    Validation { detail: String },
    Io { path: String, detail: String },
}

impl fmt::Display for JournalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JournalError::Validation { detail } => write!(f, "Journal validation: {detail}"),
            JournalError::Io { path, detail } => write!(f, "Journal I/O ({path}): {detail}"),
        }
    }
}

/// Validate required fields before write per spec.
fn validate_event(event: &JournalEvent) -> Result<(), JournalError> {
    if event.schema_version != 1 {
        return Err(JournalError::Validation {
            detail: format!("schema_version must be 1, got {}", event.schema_version),
        });
    }
    if event.event_type != "submitted" {
        return Err(JournalError::Validation {
            detail: format!("event_type must be 'submitted', got '{}'", event.event_type),
        });
    }
    if event.entry_id.is_empty() {
        return Err(JournalError::Validation {
            detail: "entry_id must be non-empty".to_string(),
        });
    }
    if event.implementation.is_empty() {
        return Err(JournalError::Validation {
            detail: "implementation must be non-empty".to_string(),
        });
    }
    if event.trigger_source != "timer" && event.trigger_source != "manual" {
        return Err(JournalError::Validation {
            detail: format!(
                "trigger_source must be 'timer' or 'manual', got '{}'",
                event.trigger_source
            ),
        });
    }
    if !event.next_interval_minutes.is_finite() || event.next_interval_minutes <= 0.0 {
        return Err(JournalError::Validation {
            detail: format!(
                "next_interval_minutes must be > 0, got {}",
                event.next_interval_minutes
            ),
        });
    }
    Ok(())
}

/// Serialize event as a single JSON line (no trailing newline).
pub fn serialize_event(event: &JournalEvent) -> Result<String, JournalError> {
    serde_json::to_string(event).map_err(|e| JournalError::Validation {
        detail: e.to_string(),
    })
}

/// Build the journal file path under a Documents-style root per spec §File location:
/// `<documents>/Nudge/journal-<impl>.ndjson`. Pure — caller resolves the
/// Documents directory via `resolve_default_journal_path` (or a test stub).
#[cfg(not(target_arch = "wasm32"))]
pub fn journal_path(documents_dir: &std::path::Path, implementation: &str) -> std::path::PathBuf {
    documents_dir
        .join("Nudge")
        .join(format!("journal-{implementation}.ndjson"))
}

/// Resolve the default Rust-implementation journal path for the current host.
/// Uses the OS-provided Documents folder (on Windows this calls the Known
/// Folder API internally via the `dirs` crate). Falls back to the current
/// working directory if the OS can't report a Documents folder.
#[cfg(not(target_arch = "wasm32"))]
pub fn resolve_default_journal_path() -> std::path::PathBuf {
    let docs = dirs::document_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    journal_path(&docs, "rust")
}

/// Append a journal event to an NDJSON file.
/// Creates parent directories if needed. Validates before write.
#[cfg(not(target_arch = "wasm32"))]
pub fn write_event(path: &std::path::Path, event: &JournalEvent) -> Result<(), JournalError> {
    use std::fs::{self, OpenOptions};
    use std::io::Write;

    let path_str = path.display().to_string();

    validate_event(event)?;

    // Ensure parent dir exists
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| JournalError::Io {
                path: path_str.clone(),
                detail: e.to_string(),
            })?;
        }
    }

    let line = serialize_event(event)?;

    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| JournalError::Io {
            path: path_str.clone(),
            detail: e.to_string(),
        })?;

    writeln!(file, "{line}").map_err(|e| JournalError::Io {
        path: path_str,
        detail: e.to_string(),
    })?;

    Ok(())
}

/// Append a journal event to localStorage (WASM).
/// Returns `JournalError::Io` on any browser-side failure (no `window`, no
/// localStorage, JS exception during read/write — e.g. `QuotaExceededError`).
#[cfg(target_arch = "wasm32")]
pub fn write_event_to_localstorage(event: &JournalEvent) -> Result<(), JournalError> {
    fn io(detail: impl Into<String>) -> JournalError {
        JournalError::Io {
            path: "localStorage:journal".to_string(),
            detail: detail.into(),
        }
    }
    fn js_err_to_string(e: wasm_bindgen::JsValue) -> String {
        e.as_string()
            .unwrap_or_else(|| format!("{:?}", e))
    }

    validate_event(event)?;
    let line = serialize_event(event)?;

    let window = web_sys::window().ok_or_else(|| io("no window"))?;
    let storage = window
        .local_storage()
        .map_err(|e| io(js_err_to_string(e)))?
        .ok_or_else(|| io("localStorage unavailable"))?;

    let existing = storage
        .get_item("journal")
        .map_err(|e| io(js_err_to_string(e)))?;
    let ndjson = match existing {
        Some(data) if !data.is_empty() => format!("{data}\n{line}"),
        _ => line,
    };

    storage
        .set_item("journal", &ndjson)
        .map_err(|e| io(js_err_to_string(e)))?;

    Ok(())
}

/// Create a new JournalEvent with all required fields for a v1 submission.
pub fn new_submitted_event(
    captured_at: String,
    trigger_source: &str,
    doing: String,
    bullshit: String,
    next_interval_minutes: f64,
) -> JournalEvent {
    JournalEvent {
        schema_version: 1,
        event_type: "submitted".to_string(),
        entry_id: ulid::Ulid::new().to_string(),
        captured_at,
        implementation: "rust".to_string(),
        trigger_source: trigger_source.to_string(),
        doing,
        bullshit,
        next_interval_minutes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_event() -> JournalEvent {
        new_submitted_event(
            "2026-04-17T14:30:00.000+02:00".to_string(),
            "timer",
            "writing tests".to_string(),
            "no".to_string(),
            10.0,
        )
    }

    // === Validation tests ===

    #[test]
    fn valid_event_passes() {
        assert!(validate_event(&test_event()).is_ok());
    }

    #[test]
    fn rejects_bad_schema_version() {
        let mut e = test_event();
        e.schema_version = 2;
        assert!(validate_event(&e).is_err());
    }

    #[test]
    fn rejects_bad_event_type() {
        let mut e = test_event();
        e.event_type = "dismissed".to_string();
        assert!(validate_event(&e).is_err());
    }

    #[test]
    fn rejects_empty_entry_id() {
        let mut e = test_event();
        e.entry_id = String::new();
        assert!(validate_event(&e).is_err());
    }

    #[test]
    fn rejects_bad_trigger_source() {
        let mut e = test_event();
        e.trigger_source = "auto".to_string();
        assert!(validate_event(&e).is_err());
    }

    #[test]
    fn rejects_zero_interval() {
        let mut e = test_event();
        e.next_interval_minutes = 0.0;
        assert!(validate_event(&e).is_err());
    }

    #[test]
    fn rejects_negative_interval() {
        let mut e = test_event();
        e.next_interval_minutes = -5.0;
        assert!(validate_event(&e).is_err());
    }

    #[test]
    fn rejects_nan_interval() {
        let mut e = test_event();
        e.next_interval_minutes = f64::NAN;
        assert!(validate_event(&e).is_err());
    }

    // === Serialization tests ===

    #[test]
    fn serialize_produces_valid_json() {
        let e = test_event();
        let line = serialize_event(&e).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["schema_version"], 1);
        assert_eq!(parsed["event_type"], "submitted");
        assert_eq!(parsed["doing"], "writing tests");
        assert_eq!(parsed["next_interval_minutes"], 10.0);
    }

    #[test]
    fn serialize_float_interval() {
        let e = new_submitted_event(
            "2026-04-17T14:30:00.000+02:00".to_string(),
            "manual",
            "test".to_string(),
            "no".to_string(),
            0.5,
        );
        let line = serialize_event(&e).unwrap();
        let parsed: serde_json::Value = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed["next_interval_minutes"], 0.5);
    }

    #[test]
    fn unicode_roundtrips() {
        let e = new_submitted_event(
            "2026-04-17T14:30:00.000+02:00".to_string(),
            "timer",
            "пишу код 🚀".to_string(),
            "нет 你好".to_string(),
            10.0,
        );
        let line = serialize_event(&e).unwrap();
        let parsed: JournalEvent = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed.doing, "пишу код 🚀");
        assert_eq!(parsed.bullshit, "нет 你好");
    }

    #[test]
    fn special_chars_roundtrip() {
        let e = new_submitted_event(
            "2026-04-17T14:30:00.000+02:00".to_string(),
            "timer",
            "tea, coffee, or \"water\"".to_string(),
            "line1\nline2".to_string(),
            10.0,
        );
        let line = serialize_event(&e).unwrap();
        let parsed: JournalEvent = serde_json::from_str(&line).unwrap();
        assert_eq!(parsed.doing, "tea, coffee, or \"water\"");
        assert_eq!(parsed.bullshit, "line1\nline2");
    }

    #[test]
    fn unknown_fields_tolerated_by_reader() {
        let json = r#"{"schema_version":1,"event_type":"submitted","entry_id":"test","captured_at":"2026-04-17T14:30:00.000+02:00","implementation":"rust","trigger_source":"timer","doing":"test","bullshit":"no","next_interval_minutes":10,"future_field":"hello"}"#;
        let parsed: Result<JournalEvent, _> = serde_json::from_str(json);
        assert!(parsed.is_ok(), "should tolerate unknown fields");
    }

    // === Path resolution tests ===

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn journal_path_joins_documents_and_nudge_subdir() {
        use std::path::{Path, PathBuf};
        let docs = Path::new("/Users/wizard/Documents");
        assert_eq!(
            journal_path(docs, "rust"),
            PathBuf::from("/Users/wizard/Documents/Nudge/journal-rust.ndjson"),
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn journal_path_uses_per_impl_filename() {
        use std::path::{Path, PathBuf};
        let docs = Path::new("/d");
        assert_eq!(
            journal_path(docs, "electron"),
            PathBuf::from("/d/Nudge/journal-electron.ndjson"),
        );
    }

    // === File tests ===

    #[cfg(not(target_arch = "wasm32"))]
    mod file_tests {
        use super::*;
        use std::fs;

        #[test]
        fn fresh_write_creates_dir_and_file() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("nested").join("deep").join("journal.ndjson");

            write_event(&path, &test_event()).unwrap();

            let content = fs::read_to_string(&path).unwrap();
            let lines: Vec<&str> = content.lines().collect();
            assert_eq!(lines.len(), 1, "should have exactly 1 line");

            let parsed: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
            assert_eq!(parsed["schema_version"], 1);
            assert_eq!(parsed["doing"], "writing tests");
        }

        #[test]
        fn two_writes_append_in_order() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("journal.ndjson");

            let e1 = new_submitted_event(
                "2026-04-17T14:30:00.000+02:00".to_string(),
                "timer",
                "first".to_string(),
                "no".to_string(),
                10.0,
            );
            let e2 = new_submitted_event(
                "2026-04-17T14:40:00.000+02:00".to_string(),
                "manual",
                "second".to_string(),
                "maybe".to_string(),
                5.0,
            );

            write_event(&path, &e1).unwrap();
            write_event(&path, &e2).unwrap();

            let content = fs::read_to_string(&path).unwrap();
            let lines: Vec<&str> = content.lines().collect();
            assert_eq!(lines.len(), 2);

            let p1: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
            let p2: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
            assert_eq!(p1["doing"], "first");
            assert_eq!(p2["doing"], "second");
        }

        #[test]
        fn write_failure_on_validation() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("journal.ndjson");

            let mut e = test_event();
            e.next_interval_minutes = 0.0;

            let result = write_event(&path, &e);
            assert!(result.is_err());
            // File should not exist
            assert!(!path.exists());
        }

        #[test]
        fn unicode_file_roundtrip() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("journal.ndjson");

            let e = new_submitted_event(
                "2026-04-17T14:30:00.000+02:00".to_string(),
                "timer",
                "пишу код 🚀".to_string(),
                "нет".to_string(),
                10.0,
            );

            write_event(&path, &e).unwrap();

            let content = fs::read_to_string(&path).unwrap();
            let parsed: JournalEvent = serde_json::from_str(content.trim()).unwrap();
            assert_eq!(parsed.doing, "пишу код 🚀");
        }

        #[test]
        fn special_chars_file_roundtrip() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("journal.ndjson");

            let e = new_submitted_event(
                "2026-04-17T14:30:00.000+02:00".to_string(),
                "timer",
                "tea, coffee, or \"water\"".to_string(),
                "line1\nline2".to_string(),
                10.0,
            );

            write_event(&path, &e).unwrap();

            let content = fs::read_to_string(&path).unwrap();
            let parsed: JournalEvent = serde_json::from_str(content.trim()).unwrap();
            assert_eq!(parsed.doing, "tea, coffee, or \"water\"");
            assert_eq!(parsed.bullshit, "line1\nline2");
        }
    }
}
