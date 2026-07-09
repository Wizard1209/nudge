//! On-disk user configuration: hotkey, default interval, autostart.
//!
//! Lives next to the journal: `<Documents>/Nudge/config.json`. We chose JSON
//! over TOML so we don't add a parser dependency (serde_json is already in
//! the tree for the journal). The file is the **source of truth** shared by
//! the main process and the settings UI (`src/settings_app.rs`, spec §9):
//! settings writes via `save()`, the main process picks up edits via the
//! watcher in `src/config_watcher.rs`. It also remains human-editable by
//! hand — that's the documented fallback when the UI isn't an option.
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

    /// Whether the app is registered to launch at OS login. Mirrors the
    /// confirmed system state (registry on Windows): the autostart provider
    /// writes this `true` only after the OS-level change succeeds. Opt-in, so
    /// it defaults to false for new installs and pre-field config files.
    #[serde(default)]
    pub autostart: bool,
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
            autostart: false,
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

/// Read/write failure shared by the native (file) and WASM (localStorage)
/// stores. `path` is the file path on native or `localStorage:<key>` on WASM
/// so the message points at the right place regardless of backend.
#[derive(Debug, Clone)]
pub enum ConfigError {
    Io { path: String, detail: String },
    Parse { path: String, detail: String },
}

impl std::fmt::Display for ConfigError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConfigError::Io { path, detail } => write!(f, "config I/O ({path}): {detail}"),
            ConfigError::Parse { path, detail } => write!(f, "config parse ({path}): {detail}"),
        }
    }
}

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

/// Pull a `--config <path>` or `--config=<path>` override out of CLI args.
/// Used by the perf test (and any future tooling) to point a launched
/// nudge.exe at a throwaway config without clobbering the user's real
/// `~/Documents/Nudge/config.json`. Returns `None` if the flag is absent
/// or trailing without a value — silent, since this is a power-user lever
/// rather than a documented contract.
#[cfg(not(target_arch = "wasm32"))]
pub fn parse_config_arg<I, S>(args: I) -> Option<std::path::PathBuf>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut iter = args.into_iter();
    while let Some(arg) = iter.next() {
        let arg_ref = arg.as_ref();
        if arg_ref == "--config" {
            return iter.next().map(|p| std::path::PathBuf::from(p.as_ref()));
        }
        if let Some(rest) = arg_ref.strip_prefix("--config=") {
            return Some(std::path::PathBuf::from(rest));
        }
    }
    None
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
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|e| ConfigError::Io {
            path: path_str.clone(),
            detail: e.to_string(),
        })?;
    }
    let body =
        serde_json::to_string_pretty(&Config::default()).map_err(|e| ConfigError::Parse {
            path: path_str.clone(),
            detail: e.to_string(),
        })?;
    fs::write(path, body).map_err(|e| ConfigError::Io {
        path: path_str,
        detail: e.to_string(),
    })
}

/// Atomically persist `config` to `path`. Writes a sibling `<path>.tmp` then
/// renames it over the target, so a concurrent reader (or a crash mid-write)
/// never observes a half-written config. Creates parent dirs as needed.
///
/// This is the write half of the source-of-truth contract: the settings
/// process saves here, the running app re-reads via `load_or_default`. The
/// temp lives in the same directory as the target so the rename stays on one
/// filesystem (cross-device renames aren't atomic). `std::fs::rename` maps to
/// `MoveFileExW(MOVEFILE_REPLACE_EXISTING)` on Windows, so it replaces an
/// existing config in place.
#[cfg(not(target_arch = "wasm32"))]
pub fn save(path: &std::path::Path, config: &Config) -> Result<(), ConfigError> {
    use std::fs;
    let path_str = path.display().to_string();

    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent).map_err(|e| ConfigError::Io {
            path: path_str.clone(),
            detail: e.to_string(),
        })?;
    }

    let body = serde_json::to_string_pretty(config).map_err(|e| ConfigError::Parse {
        path: path_str.clone(),
        detail: e.to_string(),
    })?;

    let mut tmp_os = path.as_os_str().to_owned();
    tmp_os.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp_os);

    fs::write(&tmp, body).map_err(|e| ConfigError::Io {
        path: tmp.display().to_string(),
        detail: e.to_string(),
    })?;

    fs::rename(&tmp, path).map_err(|e| {
        // A failed rename must not leave the temp file littering the dir.
        let _ = fs::remove_file(&tmp);
        ConfigError::Io {
            path: path_str,
            detail: e.to_string(),
        }
    })
}

/// localStorage key holding the persisted config in the browser build.
#[cfg(target_arch = "wasm32")]
const LOCALSTORAGE_KEY: &str = "nudge-config";

#[cfg(target_arch = "wasm32")]
fn localstorage() -> Result<web_sys::Storage, ConfigError> {
    web_sys::window()
        .and_then(|w| w.local_storage().ok())
        .flatten()
        .ok_or_else(|| ConfigError::Io {
            path: format!("localStorage:{LOCALSTORAGE_KEY}"),
            detail: "localStorage unavailable".to_string(),
        })
}

/// Load config from localStorage (WASM), or `Config::default()` if absent or
/// malformed. The browser analogue of `load_or_default`, with the same
/// forgiving rule: a missing blob is not an error; a malformed one returns the
/// default alongside the error so the caller can log it.
#[cfg(target_arch = "wasm32")]
pub fn load_from_localstorage() -> (Config, Option<ConfigError>) {
    let storage = match localstorage() {
        Ok(s) => s,
        Err(e) => return (Config::default(), Some(e)),
    };
    let raw = match storage.get_item(LOCALSTORAGE_KEY) {
        Ok(Some(s)) if !s.is_empty() => s,
        Ok(_) => return (Config::default(), None),
        Err(_) => {
            return (
                Config::default(),
                Some(ConfigError::Io {
                    path: format!("localStorage:{LOCALSTORAGE_KEY}"),
                    detail: "get_item failed".to_string(),
                }),
            );
        }
    };
    match serde_json::from_str::<Config>(&raw) {
        Ok(cfg) => (cfg, None),
        Err(e) => (
            Config::default(),
            Some(ConfigError::Parse {
                path: format!("localStorage:{LOCALSTORAGE_KEY}"),
                detail: e.to_string(),
            }),
        ),
    }
}

/// Persist config to localStorage (WASM). The browser analogue of `save`.
#[cfg(target_arch = "wasm32")]
pub fn save_to_localstorage(config: &Config) -> Result<(), ConfigError> {
    let body = serde_json::to_string(config).map_err(|e| ConfigError::Parse {
        path: format!("localStorage:{LOCALSTORAGE_KEY}"),
        detail: e.to_string(),
    })?;
    let storage = localstorage()?;
    storage
        .set_item(LOCALSTORAGE_KEY, &body)
        .map_err(|_| ConfigError::Io {
            path: format!("localStorage:{LOCALSTORAGE_KEY}"),
            detail: "set_item failed".to_string(),
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

    #[test]
    fn autostart_defaults_to_false() {
        // New installs must never silently register for autostart — opt-in only.
        assert!(!Config::default().autostart);
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
            std::fs::write(&path, r#"{"hotkey":"Ctrl+Shift+Space","future_field":42}"#).unwrap();
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
        fn parse_config_arg_absent_returns_none() {
            assert_eq!(parse_config_arg(Vec::<&str>::new()), None);
            assert_eq!(parse_config_arg(["--other", "value"]), None);
        }

        #[test]
        fn parse_config_arg_space_separated() {
            assert_eq!(
                parse_config_arg(["--config", "/tmp/foo.json"]),
                Some(std::path::PathBuf::from("/tmp/foo.json"))
            );
        }

        #[test]
        fn parse_config_arg_equals_form() {
            assert_eq!(
                parse_config_arg(["--config=/tmp/foo.json"]),
                Some(std::path::PathBuf::from("/tmp/foo.json"))
            );
        }

        #[test]
        fn parse_config_arg_with_preceding_args() {
            // The flag can be anywhere in the arg list, not just first.
            assert_eq!(
                parse_config_arg(["--verbose", "--config", "/tmp/x.json", "--quiet"]),
                Some(std::path::PathBuf::from("/tmp/x.json"))
            );
        }

        #[test]
        fn parse_config_arg_trailing_returns_none() {
            // `--config` at the end with no value: don't consume something
            // that isn't there. Silent None — the caller falls back to the
            // default config path.
            assert_eq!(parse_config_arg(["--other", "--config"]), None);
        }

        #[test]
        fn parse_config_arg_first_wins() {
            // If somebody passes --config twice, take the first one — same
            // rule as most CLI parsers. (Unlikely in practice; this just
            // pins the behavior.)
            assert_eq!(
                parse_config_arg(["--config", "/a.json", "--config", "/b.json"]),
                Some(std::path::PathBuf::from("/a.json"))
            );
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

        #[test]
        fn autostart_field_round_trips() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            std::fs::write(&path, r#"{"hotkey":"Ctrl+Shift+Space","autostart":true}"#).unwrap();
            let (cfg, err) = load_or_default(&path);
            assert!(err.is_none());
            assert!(cfg.autostart);
        }

        #[test]
        fn missing_autostart_field_defaults_false() {
            // Configs written before the autostart field existed must still
            // parse, with autostart taking its serde default of false.
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            std::fs::write(&path, r#"{"hotkey":"Alt+J"}"#).unwrap();
            let (cfg, err) = load_or_default(&path);
            assert!(err.is_none());
            assert!(!cfg.autostart);
        }

        #[test]
        fn save_round_trips_through_load() {
            // save() then load_or_default() must reproduce the exact config —
            // this is the source-of-truth contract the settings window relies on.
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            let cfg = Config {
                hotkey: "Alt+J".to_string(),
                default_interval_minutes: 7.0,
                autostart: true,
            };
            save(&path, &cfg).unwrap();
            let (loaded, err) = load_or_default(&path);
            assert!(err.is_none());
            assert_eq!(loaded, cfg);
        }

        #[test]
        fn save_creates_parent_dirs() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("nested").join("config.json");
            save(&path, &Config::default()).unwrap();
            assert!(path.exists());
        }

        #[test]
        fn save_overwrites_existing() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            save(
                &path,
                &Config {
                    autostart: false,
                    ..Config::default()
                },
            )
            .unwrap();
            save(
                &path,
                &Config {
                    autostart: true,
                    ..Config::default()
                },
            )
            .unwrap();
            let (loaded, _) = load_or_default(&path);
            assert!(loaded.autostart);
        }

        #[test]
        fn save_leaves_no_temp_file() {
            // Atomic write writes a sibling temp then renames; the temp must
            // never survive a successful save.
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            save(&path, &Config::default()).unwrap();
            let mut entries: Vec<String> = std::fs::read_dir(dir.path())
                .unwrap()
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            entries.sort();
            assert_eq!(
                entries,
                vec!["config.json".to_string()],
                "only the final file should remain"
            );
        }
    }
}
