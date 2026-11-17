use crate::config::{Config, SpriteConfig};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigChange {
    Hotkey { from: String, to: String },
    Interval,
    Autostart,
    Sprite { to: SpriteConfig },
}

pub fn diff(old: &Config, new: &Config) -> Vec<ConfigChange> {
    let mut out = Vec::new();
    if old.hotkey != new.hotkey {
        out.push(ConfigChange::Hotkey {
            from: old.hotkey.clone(),
            to: new.hotkey.clone(),
        });
    }
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

    #[derive(Debug, Clone, PartialEq)]
    pub enum LiveChange {
        Hotkey(Option<Hotkey>),
        Sprite(SpriteConfig),
    }

    pub struct LiveConfig {
        path: PathBuf,
        current: Config,
    }

    impl LiveConfig {
        pub fn new(path: PathBuf, current: Config) -> Self {
            Self { path, current }
        }

        pub fn current(&self) -> &Config {
            &self.current
        }

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

    pub struct WatcherHandle {
        _watcher: RecommendedWatcher,
    }

    pub fn spawn(
        config_path: PathBuf,
        on_change: Box<dyn Fn() + Send + Sync>,
    ) -> Result<WatcherHandle, notify::Error> {
        let parent: PathBuf = config_path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));

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
        fn spawn_keeps_firing_across_repeated_saves() {
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
            std::thread::sleep(Duration::from_millis(100));

            for (n, hotkey) in ["Alt+J", "Ctrl+F12", "Alt+K"].into_iter().enumerate() {
                let prev = counter.load(Ordering::SeqCst);
                save(
                    &path,
                    &Config {
                        hotkey: hotkey.to_string(),
                        ..Config::default()
                    },
                )
                .unwrap();
                let after = wait_for_callback(&counter, prev, Duration::from_secs(2));
                assert!(after > prev, "save #{} did not fire the watcher", n + 1);
                std::thread::sleep(Duration::from_millis(200));
            }
        }

        #[test]
        fn spawn_ignores_unrelated_files_in_same_dir() {
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

            std::thread::sleep(Duration::from_millis(500));
            counter.store(0, Ordering::SeqCst);

            let sibling = dir.path().join("journal-rust.ndjson");
            for _ in 0..3 {
                std::fs::write(&sibling, b"noise\n").unwrap();
                std::thread::sleep(Duration::from_millis(50));
            }

            std::thread::sleep(Duration::from_millis(300));
            assert_eq!(
                counter.load(Ordering::SeqCst),
                0,
                "writes to sibling files must not trigger the config callback"
            );
        }

        #[test]
        fn dropping_handle_stops_callbacks() {
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
