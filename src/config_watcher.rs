//! Live-reload of `config.json`.
//!
//! The settings process writes the file atomically (`config::save` →
//! tmp + rename). The main process needs to notice and apply edits
//! without restart. This module owns both halves of that loop:
//!
//! - **Pure diff** (`diff`, `ConfigChange`) — given the old and new
//!   `Config`, what changed? Always-compiled, fully unit-testable, the
//!   only piece with non-trivial logic.
//! - **Filesystem watcher** (`spawn`, native only) — wraps a `notify`
//!   `RecommendedWatcher` around the *parent directory* of the config
//!   file and calls a callback whenever an event references the target
//!   file name. The atomic-save dance (`config.json.tmp` → rename over
//!   `config.json`) changes the inode, so a watch attached to the file
//!   itself would die after one save on Linux; watching the directory
//!   and filtering by name is the standard fix.
//!
//! The actual `UnregisterHotKey` + `RegisterHotKey` syscalls aren't
//! here — they're queue-scoped to the tray thread (see
//! `tray_bridge::tray_thread_main`). The watcher's only job is to set a
//! "reload requested" atomic and wake the tray thread; that thread then
//! reads the new config, diffs it, and acts on each `ConfigChange`.
//!
//! Live-meaningful fields (after Tasks 1–4):
//!
//! - `hotkey` — re-register on the tray thread.
//! - `default_interval_minutes` — record-keeping only. The popup
//!   captured it at startup and the user owns the field afterwards;
//!   reloading mid-run would clobber their typing for no benefit.
//! - `autostart` — settings process owns the registry write
//!   transactionally; the main process never touches autostart on its
//!   own. Record-keeping only.

use crate::config::Config;

/// A single field-level change between two configs. Returned by `diff`
/// for every field that differs; the caller decides which ones imply
/// live UI action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigChange {
    HotkeyChanged { from: String, to: String },
    IntervalChanged,
    AutostartChanged,
}

/// Compute the set of field-level changes from `old` to `new`. Empty
/// vec when the configs are equal. Stable order: hotkey, interval,
/// autostart — matches the field order in the struct so callers can
/// rely on it for logging without sorting.
pub fn diff(old: &Config, new: &Config) -> Vec<ConfigChange> {
    let mut out = Vec::new();
    if old.hotkey != new.hotkey {
        out.push(ConfigChange::HotkeyChanged {
            from: old.hotkey.clone(),
            to: new.hotkey.clone(),
        });
    }
    // Compare interval via bit pattern so we treat NaN-vs-NaN as
    // equal — the user clearly didn't "change" it by re-saving the same
    // bogus value, and PartialEq on f64 would return false on NaN.
    if old.default_interval_minutes.to_bits() != new.default_interval_minutes.to_bits() {
        out.push(ConfigChange::IntervalChanged);
    }
    if old.autostart != new.autostart {
        out.push(ConfigChange::AutostartChanged);
    }
    out
}

// ----------------------------------------------------------------------
// Filesystem watcher — native only. The wasm build has no file system
// and the `notify` crate doesn't compile to wasm.
// ----------------------------------------------------------------------

#[cfg(not(target_arch = "wasm32"))]
#[allow(unused_imports)] // WatcherHandle is held by-value at the bin call site
                        // via type inference (`Option<WatcherHandle>`) — the
                        // bin never names the type, but it must stay public
                        // for tests and downstream consumers of the lib.
pub use native::{WatcherHandle, spawn};

#[cfg(not(target_arch = "wasm32"))]
mod native {
    use std::path::{Path, PathBuf};
    use std::sync::Arc;

    use notify::{Event, RecommendedWatcher, RecursiveMode, Watcher};

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
                let matches = event
                    .paths
                    .iter()
                    .any(|p| p.file_name().is_some_and(|n| n == file_name_for_cb.as_os_str()));
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
    use crate::config::Config;

    fn cfg_with(hotkey: &str, interval: f64, autostart: bool) -> Config {
        Config {
            hotkey: hotkey.to_string(),
            default_interval_minutes: interval,
            autostart,
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
            ConfigChange::HotkeyChanged {
                from: "Ctrl+Shift+Space".to_string(),
                to: "Alt+J".to_string(),
            }
        );
    }

    #[test]
    fn diff_detects_interval_change() {
        let a = cfg_with("Ctrl+Shift+Space", 10.0, false);
        let b = cfg_with("Ctrl+Shift+Space", 5.0, false);
        assert_eq!(diff(&a, &b), vec![ConfigChange::IntervalChanged]);
    }

    #[test]
    fn diff_detects_autostart_change() {
        let a = cfg_with("Ctrl+Shift+Space", 10.0, false);
        let b = cfg_with("Ctrl+Shift+Space", 10.0, true);
        assert_eq!(diff(&a, &b), vec![ConfigChange::AutostartChanged]);
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
        assert!(matches!(
            changes[0],
            ConfigChange::HotkeyChanged { .. }
        ));
        assert_eq!(changes[1], ConfigChange::IntervalChanged);
        assert_eq!(changes[2], ConfigChange::AutostartChanged);
    }

    #[test]
    fn diff_nan_interval_compares_equal_to_itself() {
        // `f64::NAN != f64::NAN` by IEEE 754 — but a config "re-saved"
        // with the same garbage value isn't a change. Use bit-equality
        // so we don't fire a spurious IntervalChanged on every reload
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
        fn wait_for_callback(
            counter: &Arc<AtomicUsize>,
            prev: usize,
            timeout: Duration,
        ) -> usize {
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

            std::thread::sleep(Duration::from_millis(100));

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
