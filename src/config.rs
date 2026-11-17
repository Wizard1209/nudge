use serde::{Deserialize, Serialize};

use crate::hotkey;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Config {
    #[serde(default = "default_hotkey_label")]
    pub hotkey: String,

    #[serde(default = "default_interval_minutes_value")]
    pub default_interval_minutes: f64,

    #[serde(default)]
    pub autostart: bool,

    #[serde(default, deserialize_with = "forgiving_sprite")]
    pub sprite: SpriteConfig,
}

pub const DEFAULT_CARD_WIDTH: u32 = 480;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SpriteConfig {
    pub enabled: bool,
    pub path: String,
    pub x: i32,
    pub y: i32,
    pub card_width: u32,
}

impl Default for SpriteConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            path: String::new(),
            x: 0,
            y: 0,
            card_width: DEFAULT_CARD_WIDTH,
        }
    }
}

impl SpriteConfig {
    fn from_value(raw: &serde_json::Value) -> Self {
        let default = Self::default();
        let Some(obj) = raw.as_object() else {
            return default;
        };
        Self {
            enabled: obj
                .get("enabled")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(default.enabled),
            path: obj
                .get("path")
                .and_then(serde_json::Value::as_str)
                .map_or(default.path, str::to_string),
            x: coordinate_or_default(obj.get("x").and_then(serde_json::Value::as_f64)),
            y: coordinate_or_default(obj.get("y").and_then(serde_json::Value::as_f64)),
            card_width: card_width_or_default(
                obj.get("card_width").and_then(serde_json::Value::as_f64),
            ),
        }
    }
}

fn forgiving_sprite<'de, D: serde::Deserializer<'de>>(d: D) -> Result<SpriteConfig, D::Error> {
    let raw = serde_json::Value::deserialize(d)?;
    Ok(SpriteConfig::from_value(&raw))
}

fn rounded_int(value: Option<f64>) -> Option<f64> {
    value.filter(|v| v.is_finite()).map(|v| (v + 0.5).floor())
}

fn coordinate_or_default(value: Option<f64>) -> i32 {
    rounded_int(value).map_or(0, |v| v.clamp(i32::MIN as f64, i32::MAX as f64) as i32)
}

fn card_width_or_default(value: Option<f64>) -> u32 {
    match rounded_int(value) {
        Some(w) if w > 0.0 => w.min(u32::MAX as f64) as u32,
        _ => DEFAULT_CARD_WIDTH,
    }
}

fn default_hotkey_label() -> String {
    hotkey::format(&hotkey::default_hotkey())
}

pub const DEFAULT_INTERVAL_MINUTES: f64 = 10.0;

pub const MAX_INTERVAL_MINUTES: f64 = 1_000_000.0;

pub fn valid_interval_minutes(minutes: f64) -> bool {
    minutes > 0.0 && minutes <= MAX_INTERVAL_MINUTES
}

pub fn interval_rule_text() -> String {
    format!("more than 0 and at most {MAX_INTERVAL_MINUTES} minutes")
}

#[derive(Debug, Clone, PartialEq)]
pub struct IntervalError {
    pub input: String,
}

impl std::fmt::Display for IntervalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Interval must be {} (got: \"{}\")",
            interval_rule_text(),
            hotkey::echoed(&self.input)
        )
    }
}

impl std::error::Error for IntervalError {}

fn default_interval_minutes_value() -> f64 {
    DEFAULT_INTERVAL_MINUTES
}

impl Default for Config {
    fn default() -> Self {
        Self {
            hotkey: default_hotkey_label(),
            default_interval_minutes: default_interval_minutes_value(),
            autostart: false,
            sprite: SpriteConfig::default(),
        }
    }
}

impl Config {
    pub fn hotkey_or_unbound(&self) -> Option<hotkey::Hotkey> {
        match hotkey::parse(&self.hotkey) {
            Ok(hk) => Some(hk),
            Err(e) => {
                eprintln!(
                    "[nudge] config hotkey \"{}\" is unusable ({e}), leaving it unbound. {}",
                    self.hotkey,
                    hotkey::VOCABULARY
                );
                None
            }
        }
    }

    pub fn resolved_interval_minutes(&self) -> (f64, bool) {
        if valid_interval_minutes(self.default_interval_minutes) {
            (self.default_interval_minutes, false)
        } else {
            (DEFAULT_INTERVAL_MINUTES, true)
        }
    }
}

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

#[cfg(not(target_arch = "wasm32"))]
pub fn config_path(root: &std::path::Path) -> std::path::PathBuf {
    root.join("Nudge").join("config.json")
}

#[cfg(not(target_arch = "wasm32"))]
fn resolve_default_config_path() -> std::path::PathBuf {
    config_path(&config_root())
}

#[cfg(not(target_arch = "wasm32"))]
fn config_root() -> std::path::PathBuf {
    dirs::config_dir().unwrap_or_else(|| std::path::PathBuf::from("."))
}

#[cfg(not(target_arch = "wasm32"))]
fn legacy_config_path() -> Option<std::path::PathBuf> {
    dirs::document_dir().map(|d| config_path(&d))
}

#[cfg(not(target_arch = "wasm32"))]
fn migrate_legacy_config(destination: &std::path::Path) -> std::path::PathBuf {
    match legacy_config_path() {
        Some(legacy) => {
            move_config_file(&legacy, destination, |from, to| std::fs::rename(from, to))
        }
        None => destination.to_path_buf(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn move_config_file(
    legacy: &std::path::Path,
    destination: &std::path::Path,
    rename: impl Fn(&std::path::Path, &std::path::Path) -> std::io::Result<()>,
) -> std::path::PathBuf {
    if legacy == destination || destination.exists() || !legacy.exists() {
        return destination.to_path_buf();
    }
    let moved = destination
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| rename(legacy, destination))
        .or_else(|_| move_by_copy(legacy, destination, &rename));
    match moved {
        Ok(()) => {
            eprintln!(
                "[nudge] moved config {} → {}",
                legacy.display(),
                destination.display()
            );
            destination.to_path_buf()
        }
        Err(e) => {
            eprintln!(
                "[nudge] could not move config {} → {}: {e}; using it where it is",
                legacy.display(),
                destination.display()
            );
            legacy.to_path_buf()
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn move_by_copy(
    legacy: &std::path::Path,
    destination: &std::path::Path,
    rename: impl Fn(&std::path::Path, &std::path::Path) -> std::io::Result<()>,
) -> std::io::Result<()> {
    let bytes = std::fs::read(legacy)?;
    let mut tmp = destination.as_os_str().to_owned();
    tmp.push(".tmp");
    let tmp = std::path::PathBuf::from(tmp);
    let placed = std::fs::write(&tmp, &bytes)
        .and_then(|()| std::fs::read(&tmp))
        .and_then(|copied| {
            if copied == bytes {
                rename(&tmp, destination)
            } else {
                Err(std::io::Error::other("the copy differs from the original"))
            }
        });
    if placed.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    placed?;
    if let Err(e) = std::fs::remove_file(legacy) {
        eprintln!(
            "[nudge] config copied to {}, but {} could not be removed ({e}); it is no longer read",
            destination.display(),
            legacy.display()
        );
    }
    Ok(())
}

#[cfg(not(target_arch = "wasm32"))]
pub struct ConfigFile {
    path: std::path::PathBuf,
    overridden: bool,
}

#[cfg(not(target_arch = "wasm32"))]
impl ConfigFile {
    pub fn from_args<I, S>(args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        Self::from_args_or(args, || {
            migrate_legacy_config(&resolve_default_config_path())
        })
    }

    pub(crate) fn from_args_or<I, S>(args: I, default: impl FnOnce() -> std::path::PathBuf) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        match parse_config_arg(args) {
            Some(path) => Self {
                path,
                overridden: true,
            },
            None => Self {
                path: default(),
                overridden: false,
            },
        }
    }

    pub fn path(&self) -> &std::path::Path {
        &self.path
    }

    pub fn load(&self) -> (Config, Option<ConfigError>) {
        load_or_default(&self.path)
    }

    pub fn seed_default_if_missing(&self) -> Result<(), ConfigError> {
        if self.overridden {
            return Ok(());
        }
        ensure_default_written(&self.path)
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn parse_config_arg<I, S>(args: I) -> Option<std::path::PathBuf>
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

    let json = raw.strip_prefix('\u{FEFF}').unwrap_or(&raw);
    match serde_json::from_str::<Config>(json) {
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

#[cfg(not(target_arch = "wasm32"))]
pub fn ensure_default_written(path: &std::path::Path) -> Result<(), ConfigError> {
    if path.exists() {
        return Ok(());
    }
    save(path, &Config::default())
}

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
        let _ = fs::remove_file(&tmp);
        ConfigError::Io {
            path: path_str,
            detail: e.to_string(),
        }
    })
}

#[cfg(target_arch = "wasm32")]
const LOCALSTORAGE_KEY: &str = "nudge-config";

#[cfg(target_arch = "wasm32")]
fn storage_io(detail: impl Into<String>) -> ConfigError {
    ConfigError::Io {
        path: format!("localStorage:{LOCALSTORAGE_KEY}"),
        detail: detail.into(),
    }
}

#[cfg(target_arch = "wasm32")]
fn storage_parse(detail: impl Into<String>) -> ConfigError {
    ConfigError::Parse {
        path: format!("localStorage:{LOCALSTORAGE_KEY}"),
        detail: detail.into(),
    }
}

#[cfg(target_arch = "wasm32")]
fn localstorage() -> Result<web_sys::Storage, ConfigError> {
    web_sys::window()
        .and_then(|w| w.local_storage().ok())
        .flatten()
        .ok_or_else(|| storage_io("localStorage unavailable"))
}

#[cfg(target_arch = "wasm32")]
pub fn load_from_localstorage() -> (Config, Option<ConfigError>) {
    let storage = match localstorage() {
        Ok(s) => s,
        Err(e) => return (Config::default(), Some(e)),
    };
    let raw = match storage.get_item(LOCALSTORAGE_KEY) {
        Ok(Some(s)) if !s.is_empty() => s,
        Ok(_) => return (Config::default(), None),
        Err(_) => return (Config::default(), Some(storage_io("get_item failed"))),
    };
    match serde_json::from_str::<Config>(&raw) {
        Ok(cfg) => (cfg, None),
        Err(e) => (Config::default(), Some(storage_parse(e.to_string()))),
    }
}

#[cfg(target_arch = "wasm32")]
pub fn save_to_localstorage(config: &Config) -> Result<(), ConfigError> {
    let body = serde_json::to_string(config).map_err(|e| storage_parse(e.to_string()))?;
    let storage = localstorage()?;
    storage
        .set_item(LOCALSTORAGE_KEY, &body)
        .map_err(|_| storage_io("set_item failed"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_hotkey_label_matches_parser_default() {
        let cfg = Config::default();
        assert_eq!(cfg.hotkey_or_unbound(), Some(hotkey::default_hotkey()));
    }

    #[test]
    fn resolves_user_hotkey() {
        let cfg = Config {
            hotkey: "Alt+J".to_string(),
            ..Config::default()
        };
        let hk = cfg.hotkey_or_unbound().expect("Alt+J parses");
        assert_eq!(hk.modifiers, hotkey::Modifiers::ALT);
        assert_eq!(hk.key.as_str(), "J");
    }

    #[test]
    fn invalid_hotkey_leaves_the_app_unbound() {
        let cfg = Config {
            hotkey: "Notarealkey".to_string(),
            ..Config::default()
        };
        assert_eq!(cfg.hotkey_or_unbound(), None);
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
    fn a_number_too_large_to_arm_is_refused_not_armed() {
        assert!(!valid_interval_minutes(1e20));
        assert_eq!(
            crate::settings_app::parse_interval_minutes("1e20"),
            Err(IntervalError {
                input: "1e20".to_string()
            })
        );
        assert!(crate::nudge_state::parse_interval("1e20").is_err());
    }

    #[test]
    fn the_interval_rule_ends_at_a_million_minutes() {
        assert!(valid_interval_minutes(MAX_INTERVAL_MINUTES));
        assert!(!valid_interval_minutes(MAX_INTERVAL_MINUTES.next_up()));
        assert!(std::time::Duration::try_from_secs_f64(2e17 * 60.0).is_ok());
        #[cfg(unix)]
        assert!(
            std::time::Instant::now()
                .checked_add(std::time::Duration::from_secs_f64(2e17 * 60.0))
                .is_none()
        );
        assert!(!valid_interval_minutes(2e17));
    }

    #[test]
    fn the_longest_interval_arms() {
        let longest = crate::nudge_state::parse_interval("1000000").expect("the edge is accepted");
        assert_eq!(longest.as_secs_f64(), MAX_INTERVAL_MINUTES * 60.0);
        let _deadline = web_time::Instant::now() + longest;
        let mut timer = crate::timer::Timer::frozen();
        timer.reset(longest);
        assert!(!timer.is_expired());
    }

    #[test]
    fn a_configured_interval_too_large_to_arm_falls_back() {
        for bad in [1e20, 2e17, MAX_INTERVAL_MINUTES.next_up()] {
            let cfg = Config {
                default_interval_minutes: bad,
                ..Config::default()
            };
            assert_eq!(cfg.resolved_interval_minutes(), (10.0, true), "{bad}");
        }
        let longest = Config {
            default_interval_minutes: MAX_INTERVAL_MINUTES,
            ..Config::default()
        };
        assert_eq!(
            longest.resolved_interval_minutes(),
            (MAX_INTERVAL_MINUTES, false)
        );
    }

    #[test]
    fn the_refusal_states_the_rule_it_applies() {
        assert_eq!(
            IntervalError {
                input: "1e20".to_string()
            }
            .to_string(),
            "Interval must be more than 0 and at most 1000000 minutes (got: \"1e20\")"
        );
    }

    #[test]
    fn autostart_defaults_to_false() {
        assert!(!Config::default().autostart);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn config_path_is_nudge_slash_config_under_its_root() {
        use std::path::{Path, PathBuf};
        let root = Path::new("/d");
        assert_eq!(config_path(root), PathBuf::from("/d/Nudge/config.json"));
    }

    #[cfg(all(target_os = "macos", not(target_arch = "wasm32")))]
    #[test]
    fn macos_config_lives_in_application_support() {
        let path = resolve_default_config_path();
        assert!(
            path.ends_with("Nudge/config.json"),
            "unexpected tail: {}",
            path.display()
        );
        let root = path.parent().unwrap().parent().unwrap();
        assert!(
            root.ends_with("Library/Application Support"),
            "config root must be Application Support, got {}",
            root.display()
        );
        assert_eq!(
            path,
            resolve_default_config_path(),
            "the path must not depend on anything that varies between calls"
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_config_lives_in_roaming_appdata() {
        let path = resolve_default_config_path();
        let appdata = std::path::PathBuf::from(std::env::var_os("APPDATA").expect("%APPDATA%"));
        assert_eq!(path, appdata.join("Nudge").join("config.json"));
        let docs = dirs::document_dir().expect("Documents known folder");
        assert!(
            !path.starts_with(&docs),
            "config must not live under Documents, got {}",
            path.display()
        );
    }

    #[cfg(not(target_arch = "wasm32"))]
    mod migration_tests {
        use super::*;

        fn seed(dir: &std::path::Path, body: Option<&str>) -> std::path::PathBuf {
            let path = config_path(dir);
            if let Some(body) = body {
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(&path, body).unwrap();
            }
            path
        }

        fn real_rename(from: &std::path::Path, to: &std::path::Path) -> std::io::Result<()> {
            std::fs::rename(from, to)
        }

        fn cross_device() -> std::io::Error {
            std::io::Error::from(std::io::ErrorKind::CrossesDevices)
        }

        fn files_under(dir: &std::path::Path) -> Vec<std::path::PathBuf> {
            let mut found = Vec::new();
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    found.extend(files_under(&path));
                } else {
                    found.push(path);
                }
            }
            found
        }

        #[test]
        fn a_rename_refused_across_drives_moves_by_copying() {
            let docs = tempfile::tempdir().unwrap();
            let support = tempfile::tempdir().unwrap();
            let body = r#"{"hotkey":"Alt+J","default_interval_minutes":25}"#;
            let legacy = seed(docs.path(), Some(body));
            let destination = seed(support.path(), None);
            let refuse_the_original = |from: &std::path::Path, to: &std::path::Path| {
                if from == legacy {
                    Err(cross_device())
                } else {
                    real_rename(from, to)
                }
            };

            let in_use = move_config_file(&legacy, &destination, refuse_the_original);

            assert_eq!(in_use, destination);
            assert!(
                !legacy.exists(),
                "a copy that keeps its source is not a move"
            );
            assert_eq!(std::fs::read_to_string(&destination).unwrap(), body);
            assert_eq!(files_under(support.path()), vec![destination.clone()]);
        }

        #[test]
        fn a_failed_move_runs_on_the_legacy_file_and_seeds_nothing() {
            let docs = tempfile::tempdir().unwrap();
            let support = tempfile::tempdir().unwrap();
            let body = r#"{"hotkey":"Alt+J"}"#;
            let legacy = seed(docs.path(), Some(body));
            let destination = seed(support.path(), None);

            let in_use = move_config_file(&legacy, &destination, |_, _| Err(cross_device()));

            assert_eq!(in_use, legacy, "this session reads and writes the old file");
            let file = ConfigFile {
                path: in_use,
                overridden: false,
            };
            file.seed_default_if_missing().unwrap();
            assert_eq!(file.load().0.hotkey, "Alt+J");
            assert!(
                !destination.exists(),
                "no default may appear at the destination"
            );
            assert!(
                files_under(support.path()).is_empty(),
                "no temp file left behind"
            );
            assert_eq!(std::fs::read_to_string(&legacy).unwrap(), body);

            assert_eq!(
                move_config_file(&legacy, &destination, real_rename),
                destination
            );
            assert!(!legacy.exists());
            assert_eq!(load_or_default(&destination).0.hotkey, "Alt+J");
        }

        #[test]
        fn a_legacy_config_moves_and_leaves_nothing_behind() {
            let docs = tempfile::tempdir().unwrap();
            let support = tempfile::tempdir().unwrap();
            let legacy = seed(docs.path(), Some(r#"{"hotkey":"Alt+J"}"#));
            let destination = seed(support.path(), None);

            let in_use = move_config_file(&legacy, &destination, real_rename);

            assert_eq!(in_use, destination);
            assert!(!legacy.exists(), "the old file must not survive the move");
            let (cfg, err) = load_or_default(&destination);
            assert!(err.is_none(), "{err:?}");
            assert_eq!(cfg.hotkey, "Alt+J", "the user's settings must survive");
        }

        #[test]
        fn an_existing_destination_wins_and_the_legacy_file_is_untouched() {
            let docs = tempfile::tempdir().unwrap();
            let support = tempfile::tempdir().unwrap();
            let legacy = seed(docs.path(), Some(r#"{"hotkey":"Alt+J"}"#));
            let destination = seed(support.path(), Some(r#"{"hotkey":"Ctrl+F9"}"#));

            let in_use = move_config_file(&legacy, &destination, real_rename);

            assert_eq!(in_use, destination);
            assert_eq!(load_or_default(&destination).0.hotkey, "Ctrl+F9");
            assert!(
                legacy.exists(),
                "nothing was migrated, so nothing was moved"
            );
        }

        #[test]
        fn an_explicit_config_never_runs_the_move() {
            let docs = tempfile::tempdir().unwrap();
            let support = tempfile::tempdir().unwrap();
            let legacy = seed(docs.path(), Some(r#"{"hotkey":"Alt+J"}"#));
            let destination = seed(support.path(), None);
            let given = legacy.to_str().unwrap();
            let move_it = || move_config_file(&legacy, &destination, real_rename);

            for args in [
                vec![
                    "--settings".to_owned(),
                    "--config".to_owned(),
                    given.to_owned(),
                ],
                vec!["--settings".to_owned(), format!("--config={given}")],
            ] {
                let file = ConfigFile::from_args_or(&args, move_it);
                assert_eq!(file.path(), legacy, "{args:?}");
                file.seed_default_if_missing().unwrap();
            }

            assert!(legacy.exists(), "the legacy file was moved");
            assert!(
                !destination.exists(),
                "something was written at the destination"
            );
        }

        #[test]
        fn no_legacy_file_is_not_an_error_and_creates_nothing() {
            let docs = tempfile::tempdir().unwrap();
            let support = tempfile::tempdir().unwrap();
            let legacy = seed(docs.path(), None);
            let destination = seed(support.path(), None);

            let in_use = move_config_file(&legacy, &destination, real_rename);

            assert_eq!(in_use, destination);
            assert!(
                !destination.exists(),
                "a fresh install must reach first-run seeding, not a migrated empty file"
            );
        }
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
        fn a_utf8_byte_order_mark_is_not_a_parse_error() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            std::fs::write(&path, "\u{FEFF}{\"hotkey\":\"Alt+J\"}").unwrap();

            let (cfg, err) = load_or_default(&path);
            assert!(err.is_none(), "{err:?}");
            assert_eq!(cfg.hotkey, "Alt+J");
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
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            std::fs::write(&path, r#"{"hotkey":"Alt+J"}"#).unwrap();

            let (cfg, err) = load_or_default(&path);
            assert!(err.is_none());
            assert_eq!(cfg.default_interval_minutes, 10.0);
        }

        #[test]
        fn unknown_fields_tolerated() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            std::fs::write(&path, r#"{"hotkey":"Ctrl+Shift+Space","future_field":42}"#).unwrap();
            let (cfg, err) = load_or_default(&path);
            assert!(err.is_none());
            assert_eq!(cfg.hotkey, "Ctrl+Shift+Space");
        }

        #[test]
        fn missing_field_falls_back_via_serde_default() {
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
            assert_eq!(
                parse_config_arg(["--verbose", "--config", "/tmp/x.json", "--quiet"]),
                Some(std::path::PathBuf::from("/tmp/x.json"))
            );
        }

        #[test]
        fn parse_config_arg_trailing_returns_none() {
            assert_eq!(parse_config_arg(["--other", "--config"]), None);
        }

        #[test]
        fn parse_config_arg_first_wins() {
            assert_eq!(
                parse_config_arg(["--config", "/a.json", "--config", "/b.json"]),
                Some(std::path::PathBuf::from("/a.json"))
            );
        }

        #[test]
        fn an_explicit_config_override_is_never_seeded() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("throwaway.json");
            let file = ConfigFile::from_args(["--config", path.to_str().unwrap()]);
            assert_eq!(file.path(), path.as_path());
            file.seed_default_if_missing().unwrap();
            assert!(!path.exists(), "an overridden config must not be seeded");
        }

        #[test]
        fn ensure_default_does_not_overwrite() {
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
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            std::fs::write(&path, r#"{"hotkey":"Alt+J"}"#).unwrap();
            let (cfg, err) = load_or_default(&path);
            assert!(err.is_none());
            assert!(!cfg.autostart);
        }

        #[test]
        fn save_round_trips_through_load() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            let cfg = Config {
                hotkey: "Alt+J".to_string(),
                default_interval_minutes: 7.0,
                autostart: true,
                ..Config::default()
            };
            save(&path, &cfg).unwrap();
            let (loaded, err) = load_or_default(&path);
            assert!(err.is_none());
            assert_eq!(loaded, cfg);
        }

        fn load(body: &str) -> Config {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            std::fs::write(&path, body).unwrap();
            let (cfg, err) = load_or_default(&path);
            assert!(err.is_none(), "{body}: {err:?}");
            cfg
        }

        fn sprite_on() -> SpriteConfig {
            SpriteConfig {
                enabled: true,
                path: "/a/b.png".to_string(),
                x: -41,
                y: -277,
                card_width: 679,
            }
        }

        #[test]
        fn sprite_absent_is_off() {
            assert_eq!(
                SpriteConfig::default(),
                SpriteConfig {
                    enabled: false,
                    path: String::new(),
                    x: 0,
                    y: 0,
                    card_width: DEFAULT_CARD_WIDTH,
                }
            );
            assert_eq!(DEFAULT_CARD_WIDTH, 480);
            assert_eq!(
                load(r#"{"hotkey":"Alt+J"}"#).sprite,
                SpriteConfig::default()
            );
        }

        #[test]
        fn sprite_valid_block_round_trips() {
            let cfg = load(
                r#"{"sprite":{"enabled":true,"path":"/a/b.png","x":-41,"y":-277,"card_width":679}}"#,
            );
            assert_eq!(cfg.sprite, sprite_on(), "negatives are kept");
        }

        #[test]
        fn sprite_bad_field_falls_back_alone() {
            let cfg = load(
                r#"{"hotkey":"Alt+J","sprite":{"enabled":"yes","path":7,"x":"3","y":null,"card_width":"679"}}"#,
            );
            assert_eq!(cfg.sprite, SpriteConfig::default());
            assert_eq!(cfg.hotkey, "Alt+J", "not the whole-file fallback");
        }

        #[test]
        fn sprite_non_object_is_off() {
            for bad in ["7", "null", "true", "\"on\"", "[1,2]"] {
                let cfg = load(&format!(r#"{{"hotkey":"Alt+J","sprite":{bad}}}"#));
                assert_eq!(cfg.sprite, SpriteConfig::default(), "sprite: {bad}");
                assert_eq!(cfg.hotkey, "Alt+J", "sprite: {bad}");
            }
        }

        #[test]
        fn sprite_rounds_coordinates() {
            let cfg = load(r#"{"sprite":{"x":-40.6,"y":12.4,"card_width":679.5}}"#);
            assert_eq!(
                (cfg.sprite.x, cfg.sprite.y, cfg.sprite.card_width),
                (-41, 12, 680)
            );
            assert_eq!(load(r#"{"sprite":{"x":-0.2}}"#).sprite.x, 0, "never -0");
        }

        #[test]
        fn sprite_card_width_must_be_positive() {
            for bad in ["0", "-5", "0.4"] {
                let cfg = load(&format!(r#"{{"sprite":{{"card_width":{bad}}}}}"#));
                assert_eq!(cfg.sprite.card_width, 480, "card_width: {bad}");
            }
        }

        #[test]
        fn sprite_non_finite_is_default() {
            assert_eq!(card_width_or_default(Some(f64::INFINITY)), 480);
            assert_eq!(card_width_or_default(Some(f64::NAN)), 480);
            assert_eq!(coordinate_or_default(Some(f64::NEG_INFINITY)), 0);
            assert_eq!(coordinate_or_default(Some(f64::NAN)), 0);
        }

        #[test]
        fn save_keeps_sprite() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            let cfg = Config {
                sprite: sprite_on(),
                ..Config::default()
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
