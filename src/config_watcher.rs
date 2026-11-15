//! Live-reload of `config.json`.
//!
//! The settings process writes the file atomically (`config::save` →
//! tmp + rename). The main process needs to notice and apply edits
//! without restart. This module owns that loop up to the OS calls:
//!
//! - **Pure diff** (`diff`, `ConfigChange`) — given the old and new
//!   `Config`, what changed? Always-compiled, fully unit-testable.
//! - **Filesystem watcher** (`spawn`, native only) — wraps a `notify`
//!   `RecommendedWatcher` around the *parent directory* of the config
//!   file and calls a callback whenever an event references the target
//!   file name. The atomic-save dance (`config.json.tmp` → rename over
//!   `config.json`) changes the inode, so a watch attached to the file
//!   itself would die after one save on Linux; watching the directory
//!   and filtering by name is the standard fix.
//! - **The reload** (`LiveConfig`, native only) — the baseline a running
//!   app holds, re-read on the backend's thread when the watcher's
//!   callback has raised its flag. It logs every field that changed and
//!   returns the two a backend has to act on as `LiveChange`s: the hotkey,
//!   which the backend re-registers its own way, and the sprite block,
//!   which it parks for the popup to apply while hidden (§10). The other
//!   two fields are recorded only — the popup captured the interval at
//!   launch and the user owns the field afterwards, and the settings
//!   process owns the autostart registration transactionally.

use crate::config::{Config, SpriteConfig};

/// A single field-level change between two configs. Returned by `diff`
/// for every field that differs; the caller decides which ones imply
/// live UI action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigChange {
    Hotkey {
        from: String,
        to: String,
    },
    Interval,
    Autostart,
    /// Any field of the sprite block (spec §10); one change for the whole
    /// block, since it is applied as one.
    Sprite {
        to: SpriteConfig,
    },
}

/// Compute the set of field-level changes from `old` to `new`. Empty
/// vec when the configs are equal. Stable order: hotkey, interval,
/// autostart, sprite — matches the field order in the struct so callers
/// can rely on it for logging without sorting.
pub fn diff(old: &Config, new: &Config) -> Vec<ConfigChange> {
    let mut out = Vec::new();
    if old.hotkey != new.hotkey {
        out.push(ConfigChange::Hotkey {
            from: old.hotkey.clone(),
            to: new.hotkey.clone(),
        });
    }
    // Compare interval via bit pattern so we treat NaN-vs-NaN as
    // equal — the user clearly didn't "change" it by re-saving the same
    // bogus value, and PartialEq on f64 would return false on NaN.
    if old.default_interval_minutes.to_bits() != new.default_interval_minutes.to_bits() {
        out.push(ConfigChange::Interval);
    }
    if old.autostart != new.autostart {
        out.push(ConfigChange::Autostart);
    }
    if old.sprite != new.sprite {
        out.push(ConfigChange::Sprite {
            to: new.sprite.clone(),
        });
    }
    out
}

// ----------------------------------------------------------------------
// Filesystem watcher — native only. The wasm build has no file system
// and the `notify` crate doesn't compile to wasm.
// ----------------------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
pub use native::{LiveChange, LiveConfig, WatcherHandle, spawn};

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

    use super::{ConfigChange, diff};
    use crate::config::{self, Config, SpriteConfig};
    use crate::hotkey::Hotkey;

    /// A change a running app has to act on; everything else the file can
    /// change is logged by [`LiveConfig::reload`] and has no live effect.
    #[derive(Debug, Clone, PartialEq)]
    pub enum LiveChange {
        /// Bind this combination in place of the current one — or none,
        /// when the new label does not parse: unbound, not a fallback (§5).
        Hotkey(Option<Hotkey>),
        /// The sprite block changed; the popup applies it while hidden (§10).
        Sprite(SpriteConfig),
    }

    /// The config as the running app has it: where the file is, and what is
    /// currently in effect — the baseline every reload diffs against.
    pub struct LiveConfig {
        path: PathBuf,
        current: Config,
    }

    impl LiveConfig {
        pub fn new(path: PathBuf, current: Config) -> Self {
            Self { path, current }
        }

        /// What is currently in effect. Read by the backends' tests to check
        /// the baseline a reload left behind.
        pub fn current(&self) -> &Config {
            &self.current
        }

        /// Re-read the file, log every field that changed and return the
        /// changes the backend has to act on, in field order.
        ///
        /// A malformed file is tolerated the way `load_or_default` does it:
        /// log, keep the previous config, return nothing. The watcher must
        /// not be the thing that kills the app over a typo.
        pub fn reload(&mut self) -> Vec<LiveChange> {
            let (new_cfg, err) = config::load_or_default(&self.path);
            if let Some(e) = err {
                eprintln!("[nudge] config reload: {e} — keeping previous in-memory config");
                return Vec::new();
            }

            let mut live = Vec::new();
            for change in diff(&self.current, &new_cfg) {
                match change {
                    ConfigChange::Hotkey { from, to } => {
                        eprintln!("[nudge] config reload: hotkey \"{from}\" → \"{to}\"");
                        live.push(LiveChange::Hotkey(new_cfg.hotkey_or_unbound()));
                    }
                    ConfigChange::Interval => eprintln!(
                        "[nudge] config reload: default_interval_minutes {} → {} \
                         (no live effect; {})",
                        self.current.default_interval_minutes,
                        new_cfg.default_interval_minutes,
                        match new_cfg.resolved_interval_minutes() {
                            (_, false) => "applies at next startup".to_string(),
                            (fallback, true) => format!(
                                "not {}, so the next startup falls back to {fallback}",
                                config::interval_rule_text()
                            ),
                        }
                    ),
                    ConfigChange::Autostart => eprintln!(
                        "[nudge] config reload: autostart {} → {} \
                         (the settings process owns the OS registration)",
                        self.current.autostart, new_cfg.autostart
                    ),
                    ConfigChange::Sprite { to } => {
                        eprintln!("[nudge] config reload: sprite → {to:?} (applied while hidden)");
                        live.push(LiveChange::Sprite(to));
                    }
                }
            }
            self.current = new_cfg;
            live
        }
    }

    /// Opaque handle to a running watcher. Dropping it stops the
    /// background thread and releases the OS handle (inotify watch /
    /// ReadDirectoryChangesW handle). Keep it alive for as long as you
    /// want live-reload to work — typically the whole process lifetime.
    pub struct WatcherHandle {
        // `RecommendedWatcher` is the kernel-blocking watcher; on drop
        // it stops the background thread. Held as `Option` so we can
        // explicitly drop it in tests if we want, but normally it just
        // lives until the handle does.
        _watcher: RecommendedWatcher,
    }

    /// Start watching `config_path` and invoke `on_change` every time
    /// the file is modified, created, or atomically replaced. Returns a
    /// handle that must be kept alive — dropping it stops the watcher.
    ///
    /// We watch the *parent directory* rather than the file itself
    /// because `config::save` writes `<path>.tmp` then renames it over
    /// `<path>`, which swaps the inode under any file-level watch. The
    /// parent-directory watch sees both the tmp create and the final
    /// rename and survives across saves.
    ///
    /// `on_change` is called from the notify background thread on every
    /// event whose path component matches the config file name. We
    /// don't try to debounce here — the only consumer is the tray
    /// thread, which idempotently re-reads the file and diffs; a
    /// double-fire is harmless. Adding a debounce would mean carrying
    /// the `notify::Debouncer` machinery and a tokio-ish event-time
    /// model, which is overkill for one user-edited file.
    pub fn spawn(
        config_path: PathBuf,
        on_change: Box<dyn Fn() + Send + Sync>,
    ) -> Result<WatcherHandle, notify::Error> {
        let parent: PathBuf = config_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));

        // Filter: only fire for events that actually mention the
        // config file by name. Other dotfiles in the same directory
        // (e.g. the journal NDJSON) must not trigger a reload.
        let file_name = config_path
            .file_name()
            .map(|s| s.to_os_string())
            .unwrap_or_default();
        let file_name = Arc::new(file_name);

        let on_change: Arc<dyn Fn() + Send + Sync> = Arc::from(on_change);

        let file_name_for_cb = Arc::clone(&file_name);
        let on_change_for_cb = Arc::clone(&on_change);

        let mut watcher: RecommendedWatcher =
            notify::recommended_watcher(move |res: notify::Result<Event>| {
                let Ok(event) = res else {
                    // notify reports an error (rare — usually a
                    // permissions hiccup or the directory disappeared);
                    // don't spam the callback, just swallow.
                    return;
                };
                let matches = event.paths.iter().any(|p| {
                    p.file_name()
                        .is_some_and(|n| n == file_name_for_cb.as_os_str())
                });
                if matches {
                    (on_change_for_cb)();
                }
            })?;

        watcher.watch(&parent, RecursiveMode::NonRecursive)?;

        Ok(WatcherHandle { _watcher: watcher })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, SpriteConfig};

    fn cfg_with(hotkey: &str, interval: f64, autostart: bool) -> Config {
        Config {
            hotkey: hotkey.to_string(),
            default_interval_minutes: interval,
            autostart,
            ..Config::default()
        }
    }

    #[test]
    fn diff_empty_when_equal() {
        let a = Config::default();
        let b = Config::default();
        assert!(diff(&a, &b).is_empty());
    }

    #[test]
    fn diff_detects_hotkey_change() {
        let a = cfg_with("Ctrl+Shift+Space", 10.0, false);
        let b = cfg_with("Alt+J", 10.0, false);
        let changes = diff(&a, &b);
        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0],
            ConfigChange::Hotkey {
                from: "Ctrl+Shift+Space".to_string(),
                to: "Alt+J".to_string(),
            }
        );
    }

    #[test]
    fn diff_detects_interval_change() {
        let a = cfg_with("Ctrl+Shift+Space", 10.0, false);
        let b = cfg_with("Ctrl+Shift+Space", 5.0, false);
        assert_eq!(diff(&a, &b), vec![ConfigChange::Interval]);
    }

    #[test]
    fn diff_detects_autostart_change() {
        let a = cfg_with("Ctrl+Shift+Space", 10.0, false);
        let b = cfg_with("Ctrl+Shift+Space", 10.0, true);
        assert_eq!(diff(&a, &b), vec![ConfigChange::Autostart]);
    }

    #[test]
    fn diff_detects_multiple_fields() {
        // All three fields change at once — verify both presence and
        // the stable order documented in the doc comment (hotkey,
        // interval, autostart).
        let a = cfg_with("Ctrl+Shift+Space", 10.0, false);
        let b = cfg_with("Alt+J", 5.0, true);
        let changes = diff(&a, &b);
        assert_eq!(changes.len(), 3);
        assert!(matches!(changes[0], ConfigChange::Hotkey { .. }));
        assert_eq!(changes[1], ConfigChange::Interval);
        assert_eq!(changes[2], ConfigChange::Autostart);
    }

    #[test]
    fn diff_flags_each_sprite_field() {
        let base = Config {
            sprite: SpriteConfig {
                enabled: true,
                path: "/a/b.png".to_string(),
                x: -41,
                y: -277,
                card_width: 679,
            },
            ..Config::default()
        };
        let s = &base.sprite;
        let changed = [
            (
                "enabled",
                SpriteConfig {
                    enabled: false,
                    ..s.clone()
                },
            ),
            (
                "path",
                SpriteConfig {
                    path: "/c.png".to_string(),
                    ..s.clone()
                },
            ),
            ("x", SpriteConfig { x: 0, ..s.clone() }),
            ("y", SpriteConfig { y: 1, ..s.clone() }),
            (
                "card_width",
                SpriteConfig {
                    card_width: 480,
                    ..s.clone()
                },
            ),
        ];
        for (field, sprite) in changed {
            let next = Config {
                sprite,
                ..base.clone()
            };
            assert_eq!(
                diff(&base, &next),
                vec![ConfigChange::Sprite {
                    to: next.sprite.clone()
                }],
                "{field} changed alone"
            );
        }
        assert!(diff(&base, &base.clone()).is_empty(), "identical copy");
        let hotkey_only = Config {
            hotkey: "Alt+J".to_string(),
            ..base.clone()
        };
        assert!(
            !diff(&base, &hotkey_only)
                .iter()
                .any(|c| matches!(c, ConfigChange::Sprite { .. })),
            "a hotkey change alone is not a sprite change"
        );
    }

    #[test]
    fn diff_nan_interval_compares_equal_to_itself() {
        // `f64::NAN != f64::NAN` by IEEE 754 — but a config "re-saved"
        // with the same garbage value isn't a change. Use bit-equality
        // so we don't fire a spurious Interval on every reload
        // when the file happens to hold NaN.
        let a = cfg_with("X", f64::NAN, false);
        let b = cfg_with("X", f64::NAN, false);
        assert!(diff(&a, &b).is_empty());
    }

    #[cfg(not(target_arch = "wasm32"))]
    mod watcher_tests {
        use super::super::*;
        use crate::config::{Config, save};
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::{Duration, Instant};

        /// Atomically save `cfg` and wait up to `timeout` for the
        /// callback counter to advance past `prev`. Returns the new
        /// counter value. Times out by returning the (un-advanced)
        /// prev — the assertion in the caller surfaces the failure.
        fn wait_for_callback(counter: &Arc<AtomicUsize>, prev: usize, timeout: Duration) -> usize {
            let start = Instant::now();
            loop {
                let cur = counter.load(Ordering::SeqCst);
                if cur > prev {
                    return cur;
                }
                if start.elapsed() > timeout {
                    return cur;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        }

        #[test]
        fn a_reload_of_an_interval_too_large_to_arm_changes_nothing_live() {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            let edited = Config {
                default_interval_minutes: 1e20,
                ..Config::default()
            };
            save(&path, &edited).unwrap();

            let mut live = LiveConfig::new(path, Config::default());
            assert_eq!(live.reload(), Vec::new(), "the interval has no live effect");
            assert_eq!(live.current(), &edited, "but it is the new baseline");
            assert_eq!(live.current().resolved_interval_minutes(), (10.0, true));
        }

        #[test]
        fn spawn_fires_on_config_save() {
            // The whole point of the watcher: a config::save() must wake
            // us up so the tray thread can re-read and diff. Drive it
            // through the real notify backend on the host filesystem.
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            // Seed the file so the watcher target exists. Not strictly
            // required (we watch the parent dir), but matches the
            // production order: main loads config, *then* spawns the
            // watcher.
            save(&path, &Config::default()).unwrap();

            let counter = Arc::new(AtomicUsize::new(0));
            let counter_for_cb = Arc::clone(&counter);
            let _handle = spawn(
                path.clone(),
                Box::new(move || {
                    counter_for_cb.fetch_add(1, Ordering::SeqCst);
                }),
            )
            .expect("watcher spawn");

            // Give notify a beat to attach its inotify / kqueue / etc.
            // watch on the parent dir before we touch the file.
            std::thread::sleep(Duration::from_millis(100));

            let prev = counter.load(Ordering::SeqCst);
            save(
                &path,
                &Config {
                    hotkey: "Alt+J".to_string(),
                    ..Config::default()
                },
            )
            .unwrap();

            let after = wait_for_callback(&counter, prev, Duration::from_secs(2));
            assert!(
                after > prev,
                "watcher should have fired within 2s of config::save (counter was {prev}, now {after})"
            );
        }

        #[test]
        fn spawn_ignores_unrelated_files_in_same_dir() {
            // The journal lives next to config.json. Writing to it
            // must not trigger a config reload — that's wasted work and
            // could mask real reload bugs.
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            save(&path, &Config::default()).unwrap();

            let counter = Arc::new(AtomicUsize::new(0));
            let counter_for_cb = Arc::clone(&counter);
            let _handle = spawn(
                path.clone(),
                Box::new(move || {
                    counter_for_cb.fetch_add(1, Ordering::SeqCst);
                }),
            )
            .expect("watcher spawn");

            // Let the watcher settle, then zero the counter. FSEvents
            // delivers events that predate the stream: the `save` above
            // touched config.json, and on a loaded machine that
            // notification arrives after `spawn` has returned. Counting
            // from zero here is what makes the assertion below about
            // sibling writes rather than about how fast this host is.
            std::thread::sleep(Duration::from_millis(500));
            counter.store(0, Ordering::SeqCst);

            // Touch a sibling file repeatedly — must not wake the
            // callback.
            let sibling = dir.path().join("journal-rust.ndjson");
            for _ in 0..3 {
                std::fs::write(&sibling, b"noise\n").unwrap();
                std::thread::sleep(Duration::from_millis(50));
            }

            // Give a small grace window in case the platform batches
            // events. We expect zero callbacks.
            std::thread::sleep(Duration::from_millis(300));
            assert_eq!(
                counter.load(Ordering::SeqCst),
                0,
                "writes to sibling files must not trigger the config callback"
            );
        }

        #[test]
        fn dropping_handle_stops_callbacks() {
            // Live-reload must stop when the handle is dropped — if it
            // didn't, restart-on-config-change scenarios (or tests like
            // this one) would leak callback threads.
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("config.json");
            save(&path, &Config::default()).unwrap();

            let counter = Arc::new(AtomicUsize::new(0));
            let counter_for_cb = Arc::clone(&counter);
            let handle = spawn(
                path.clone(),
                Box::new(move || {
                    counter_for_cb.fetch_add(1, Ordering::SeqCst);
                }),
            )
            .expect("watcher spawn");
            std::thread::sleep(Duration::from_millis(100));

            // First save should fire.
            let prev = counter.load(Ordering::SeqCst);
            save(
                &path,
                &Config {
                    hotkey: "Alt+J".to_string(),
                    ..Config::default()
                },
            )
            .unwrap();
            let after_save = wait_for_callback(&counter, prev, Duration::from_secs(2));
            assert!(after_save > prev, "pre-drop save should have fired");

            drop(handle);
            std::thread::sleep(Duration::from_millis(100));

            // After drop, subsequent saves must not advance the
            // counter. We can't strictly distinguish "no callback ever"
            // from "callback raced after drop", but a quiet observation
            // window is a strong signal.
            let baseline = counter.load(Ordering::SeqCst);
            save(
                &path,
                &Config {
                    hotkey: "Ctrl+F12".to_string(),
                    ..Config::default()
                },
            )
            .unwrap();
            std::thread::sleep(Duration::from_millis(500));
            assert_eq!(
                counter.load(Ordering::SeqCst),
                baseline,
                "no callback should fire after the handle is dropped"
            );
        }
    }
}
