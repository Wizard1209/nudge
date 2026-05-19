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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Config {
    /// Hotkey label, e.g. "Ctrl+Shift+Space". Parsed lazily so an invalid
    /// label in the file doesn't refuse the whole config.
    #[serde(default = "default_hotkey_label")]
    pub hotkey: String,
}

fn default_hotkey_label() -> String {
    hotkey::format(&hotkey::default_hotkey())
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: default_hotkey_label(),
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
        };
        let (hk, invalid) = cfg.resolved_hotkey();
        assert!(invalid);
        assert_eq!(hk, hotkey::default_hotkey());
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
