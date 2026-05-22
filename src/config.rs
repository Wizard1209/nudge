//! On-disk user configuration. Currently just the global hotkey label.
//!
//! Lives next to the journal: `<Documents>/Nudge/config.json`. We chose JSON
//! over TOML so we don't add a parser dependency (serde_json is already in
//! the tree for the journal). The file format is intentionally
//! human-editable — the readme tells users to edit it by hand; there's no
//! settings UI yet.
//!
//! Loading is forgiving: a missing file produces a default config, a
//! malformed file logs to stderr and falls back to the default. We never
//! refuse to start over a bad config — that would brick a hotkey-less app
//! launch just to surface a JSON error nobody can read past the splash.

use serde::{Deserialize, Serialize};

use crate::hotkey;

/// Persisted configuration. Add fields conservatively — every field must
/// have a serde default so old config files keep parsing after we grow new
/// settings.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    /// Hotkey label, e.g. "Ctrl+Shift+Space". Parsed lazily so an invalid
    /// label in the file doesn't refuse the whole config.
    #[serde(default = "default_hotkey_label")]
    pub hotkey: String,

    /// Default interval (in minutes) shown in the popup's "next nudge"
    /// field and used to arm the first timer. Validated lazily via
    /// `resolved_interval_minutes` — see that method for the fallback rule.
    #[serde(default = "default_interval_minutes_value")]
    pub default_interval_minutes: f64,
}

fn default_hotkey_label() -> String {
    hotkey::format(&hotkey::default_hotkey())
}

fn default_interval_minutes_value() -> f64 {
    10.0
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: default_hotkey_label(),
            default_interval_minutes: default_interval_minutes_value(),
        }
    }
}

impl Config {
    /// Parse the hotkey label, falling back to the default on error.
    /// Returns `(parsed, label_was_invalid)` so the caller can log a warning
    /// without re-parsing.
    pub fn resolved_hotkey(&self) -> (hotkey::Hotkey, bool) {
        match hotkey::parse(&self.hotkey) {
            Ok(h) => (h, false),
            Err(_) => (hotkey::default_hotkey(), true),
        }
    }

    /// Validate the configured interval, falling back to the built-in
    /// default if it isn't a finite positive number. Returns
    /// `(minutes, was_invalid)` so the caller can log a warning without
    /// re-validating. Mirrors the same forgiving philosophy as
    /// `resolved_hotkey` — a garbage value in the file must never refuse
    /// the whole config.
    pub fn resolved_interval_minutes(&self) -> (f64, bool) {
        if self.default_interval_minutes.is_finite() && self.default_interval_minutes > 0.0 {
            (self.default_interval_minutes, false)
        } else {
            (default_interval_minutes_value(), true)
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug, Clone)]
pub enum ConfigError {
    Io { path: String, detail: String },
    Parse { path: String, detail: String },
}

#[cfg(not(target_arch = "wasm32"))]
impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io { path, detail } => write!(f, "config I/O ({path}): {detail}"),
            ConfigError::Parse { path, detail } => write!(f, "config parse ({path}): {detail}"),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl std::error::Error for ConfigError {}

/// Default config path: `<Documents>/Nudge/config.json`. Same root as the
/// journal so users have one folder to look in.
#[cfg(not(target_arch = "wasm32"))]
pub fn config_path(documents_dir: &std::path::Path) -> std::path::PathBuf {
    documents_dir.join("Nudge").join("config.json")
}

#[cfg(not(target_arch = "wasm32"))]
pub fn resolve_default_config_path() -> std::path::PathBuf {
    let docs = dirs::document_dir().unwrap_or_else(|| std::path::PathBuf::from("."));
    config_path(&docs)
}

/// Load config from `path`, or return `Config::default()` if the file
/// doesn't exist or doesn't parse. Errors are returned alongside the
/// fallback so the caller can log them — we never refuse to start.
#[cfg(not(target_arch = "wasm32"))]
pub fn load_or_default(path: &std::path::Path) -> (Config, Option<ConfigError>) {
    use std::fs;
    let path_str = path.display().to_string();

    let raw = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Config::default(), None),
        Err(e) => {
            return (
                Config::default(),
                Some(ConfigError::Io {
                    path: path_str,
                    detail: e.to_string(),
                }),
            );
        }
    };

    match serde_json::from_str::<Config>(&raw) {
        Ok(cfg) => (cfg, None),
        Err(e) => (
            Config::default(),
            Some(ConfigError::Parse {
                path: path_str,
                detail: e.to_string(),
            }),
        ),
    }
}

/// Write the default config to `path` if no file is there yet. Used on
/// first run so users have a starting template to edit instead of needing
/// to know the schema.
#[cfg(not(target_arch = "wasm32"))]
pub fn ensure_default_written(path: &std::path::Path) -> Result<(), ConfigError> {
    use std::fs;
    if path.exists() {
        return Ok(());
    }
    let path_str = path.display().to_string();
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            fs::create_dir_all(parent).map_err(|e| ConfigError::Io {
                path: path_str.clone(),
                detail: e.to_string(),
            })?;
        }
    }
    let body = serde_json::to_string_pretty(&Config::default()).map_err(|e| ConfigError::Parse {
        path: path_str.clone(),
        detail: e.to_string(),
    })?;
    fs::write(path, body).map_err(|e| ConfigError::Io {
        path: path_str,
        detail: e.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_hotkey_label_matches_parser_default() {
        let cfg = Config::default();
        let (hk, invalid) = cfg.resolved_hotkey();
        assert!(!invalid);
        assert_eq!(hk, hotkey::default_hotkey());
    }

    #[test]
    fn resolves_user_hotkey() {
        let cfg = Config {
            hotkey: "Alt+J".to_string(),
            ..Config::default()
        };
        let (hk, invalid) = cfg.resolved_hotkey();
        assert!(!invalid);
        assert_eq!(hk.modifiers, hotkey::MOD_ALT);
        assert_eq!(hk.key.as_str(), "J");
    }

    #[test]
    fn invalid_hotkey_falls_back_silently() {
        // Garbage in the label must not refuse the config; the global hotkey
        // just defaults and the caller is responsible for telling the user.
        let cfg = Config {
            hotkey: "Notarealkey".to_string(),
            ..Config::default()
        };
        let (hk, invalid) = cfg.resolved_hotkey();
        assert!(invalid);
        assert_eq!(hk, hotkey::default_hotkey());
    }

    #[test]
    fn default_interval_minutes_is_ten() {
        let cfg = Config::default();
        assert_eq!(cfg.default_interval_minutes, 10.0);
        let (mins, invalid) = cfg.resolved_interval_minutes();
        assert!(!invalid);
        assert_eq!(mins, 10.0);
    }

    #[test]
    fn resolves_explicit_interval() {
        let cfg = Config {
            default_interval_minutes: 5.0,
            ..Config::default()
        };
        let (mins, invalid) = cfg.resolved_interval_minutes();
        assert!(!invalid);
        assert_eq!(mins, 5.0);
    }

    #[test]
    fn resolves_fractional_interval() {
        // Sub-minute intervals are valid — useful for tests and impatient
        // users. Just needs to be a finite positive number.
        let cfg = Config {
            default_interval_minutes: 0.5,
            ..Config::default()
        };
        let (mins, invalid) = cfg.resolved_interval_minutes();
        assert!(!invalid);
        assert_eq!(mins, 0.5);
    }

    #[test]
    fn invalid_interval_falls_back_silently() {
        // Anything not a finite positive number: zero, negative, NaN, infinity.
        // All must fall back to the built-in default rather than refusing the
        // config — same forgiving rule as the hotkey field.
        for bad in [0.0, -3.0, f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            let cfg = Config {
                default_interval_minutes: bad,
                ..Config::default()
            };
            let (mins, invalid) = cfg.resolved_interval_minutes();
            assert!(invalid, "{bad} should be flagged invalid");
            assert_eq!(mins, 10.0, "{bad} should fall back to 10.0");
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn config_path_under_documents_nudge() {
        use std::path::{Path, PathBuf};
        let docs = Path::new("/d");
        assert_eq!(config_path(docs), PathBuf::from("/d/Nudge/config.json"));
    }

    #[cfg(not(target_arch = "wasm32"))]
    mod file_tests {
        use super::*;

        #[test]
        fn missing_file_returns_default() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("does-not-exist.json");
            let (cfg, err) = load_or_default(&path);
            assert_eq!(cfg, Config::default());
            assert!(err.is_none(), "missing file is not an error");
        }

        #[test]
        fn malformed_file_returns_default_with_error() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            std::fs::write(&path, "{ not json").unwrap();

            let (cfg, err) = load_or_default(&path);
            assert_eq!(cfg, Config::default());
            assert!(matches!(err, Some(ConfigError::Parse { .. })));
        }

        #[test]
        fn valid_file_round_trips() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            std::fs::write(&path, r#"{"hotkey":"Alt+J"}"#).unwrap();

            let (cfg, err) = load_or_default(&path);
            assert!(err.is_none());
            assert_eq!(cfg.hotkey, "Alt+J");
        }

        #[test]
        fn interval_field_round_trips() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            std::fs::write(
                &path,
                r#"{"hotkey":"Ctrl+Shift+Space","default_interval_minutes":7}"#,
            )
            .unwrap();

            let (cfg, err) = load_or_default(&path);
            assert!(err.is_none());
            assert_eq!(cfg.default_interval_minutes, 7.0);
        }

        #[test]
        fn missing_interval_field_uses_default() {
            // A config file written before the interval field existed must
            // still parse, with the new field taking its serde default.
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            std::fs::write(&path, r#"{"hotkey":"Alt+J"}"#).unwrap();

            let (cfg, err) = load_or_default(&path);
            assert!(err.is_none());
            assert_eq!(cfg.default_interval_minutes, 10.0);
        }

        #[test]
        fn unknown_fields_tolerated() {
            // Forward-compat: future fields shouldn't refuse the file.
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            std::fs::write(
                &path,
                r#"{"hotkey":"Ctrl+Shift+Space","future_field":42}"#,
            )
            .unwrap();
            let (cfg, err) = load_or_default(&path);
            assert!(err.is_none());
            assert_eq!(cfg.hotkey, "Ctrl+Shift+Space");
        }

        #[test]
        fn missing_field_falls_back_via_serde_default() {
            // Empty object: serde fills `hotkey` from the field default.
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            std::fs::write(&path, "{}").unwrap();
            let (cfg, err) = load_or_default(&path);
            assert!(err.is_none());
            assert_eq!(cfg, Config::default());
        }

        #[test]
        fn ensure_default_writes_when_missing() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("nested").join("config.json");
            ensure_default_written(&path).unwrap();
            assert!(path.exists(), "should have created the file");

            let (cfg, err) = load_or_default(&path);
            assert!(err.is_none());
            assert_eq!(cfg, Config::default());
        }

        #[test]
        fn ensure_default_does_not_overwrite() {
            // User edits get preserved on subsequent launches.
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            std::fs::write(&path, r#"{"hotkey":"Alt+J"}"#).unwrap();
            ensure_default_written(&path).unwrap();
            let body = std::fs::read_to_string(&path).unwrap();
            assert!(body.contains("Alt+J"), "user value preserved");
        }
    }
}
