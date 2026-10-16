//! Single-instance guard for the settings window — the macOS counterpart of
//! the named Win32 mutex in `main.rs`.
//!
//! The tray menu's "Settings" item spawns a process per click, so without a
//! guard a user drumming on the menu gets a stack of identical windows, each
//! with its own unsynchronised copy of `config.json` and a last-writer-wins
//! race between them.
//!
//! `flock` rather than a bare `O_EXCL` create, because the two differ exactly
//! where it matters: an `O_EXCL` sentinel outlives the process that made it, so
//! one crash (or one `kill -9`) leaves a file that locks the settings window
//! out until someone deletes it by hand. A `flock` is held by the open file
//! description and the kernel drops it when the process dies, however it dies.
//! The file itself is allowed to persist; it carries no state.

use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

/// Permission to be the settings window. Holding the [`File`] is what holds
/// the `flock`, so this must stay alive for as long as the window does;
/// dropping it (or exiting) releases the lock.
///
/// The `Option` is the "we could not tell" case — see
/// [`acquire_settings_singleton`].
#[derive(Debug)]
pub struct SettingsLock {
    /// Held, never read: the `flock` lives on this open file description and
    /// the kernel releases it when the description is closed.
    _file: Option<File>,
}

/// `~/Library/Application Support/Nudge/settings.lock`.
///
/// `dirs::config_dir()` is `~/Library/Application Support` on macOS — the same
/// resolution the rest of the app uses, so the lock lands beside the app's
/// other per-user state rather than in a location of its own invention.
fn settings_lock_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("Nudge").join("settings.lock"))
}

/// Take the settings-window lock, or report that another window already holds
/// it.
///
/// Returns `Ok(None)` for the ordinary "already open" case — the caller exits
/// silently, the way the Windows `ERROR_ALREADY_EXISTS` arm does. `Err` is a
/// real filesystem failure.
fn acquire_lock(path: &Path) -> io::Result<Option<SettingsLock>> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;

    // SAFETY: `fd` is owned by `file` and outlives the call.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        return Ok(Some(SettingsLock { _file: Some(file) }));
    }
    let err = io::Error::last_os_error();
    match err.raw_os_error() {
        Some(libc::EWOULDBLOCK) => Ok(None),
        _ => Err(err),
    }
}

/// Take the lock at the real path. `None` means another settings window is
/// already open and this process should exit silently — the same contract the
/// Windows `ERROR_ALREADY_EXISTS` arm has.
///
/// A filesystem error yields an unheld lock rather than `None`: it means we
/// could not tell, and turning an unwritable `Application Support` into "the
/// Settings menu item does nothing" is a worse failure than the duplicate
/// window the guard exists to prevent.
pub fn acquire_settings_singleton() -> Option<SettingsLock> {
    let Some(path) = settings_lock_path() else {
        eprintln!("[nudge-settings] no config directory — single-instance guard skipped");
        return Some(SettingsLock { _file: None });
    };
    match acquire_lock(&path) {
        Ok(Some(lock)) => Some(lock),
        Ok(None) => None,
        Err(e) => {
            eprintln!("[nudge-settings] single-instance guard unavailable: {e}");
            Some(SettingsLock { _file: None })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_acquire_succeeds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.lock");
        assert!(acquire_lock(&path).unwrap().is_some());
    }

    #[test]
    fn second_acquire_is_refused_while_the_first_is_held() {
        // The whole point of the guard. `flock` is per open file description,
        // so two `open`s of the same path contend even inside one process —
        // which is what makes this testable without spawning anything.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.lock");

        let first = acquire_lock(&path).unwrap();
        assert!(first.is_some(), "first window takes the lock");
        assert!(
            acquire_lock(&path).unwrap().is_none(),
            "second window must be refused, not queued"
        );
    }

    #[test]
    fn releasing_lets_the_next_window_in() {
        // The property `O_EXCL` would not have: closing the door does not
        // leave it locked behind you.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.lock");

        let first = acquire_lock(&path).unwrap();
        assert!(first.is_some());
        drop(first);

        assert!(
            acquire_lock(&path).unwrap().is_some(),
            "the lock must be reusable after the holder exits"
        );
    }

    #[test]
    fn a_leftover_lock_file_does_not_lock_anyone_out() {
        // Simulates the crash case: the file is still there, nobody holds it.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.lock");
        std::fs::write(&path, b"stale").unwrap();

        assert!(
            acquire_lock(&path).unwrap().is_some(),
            "a stale lock file must not be a permanent lockout"
        );
    }

    #[test]
    fn missing_directory_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Nudge").join("settings.lock");
        assert!(acquire_lock(&path).unwrap().is_some());
        assert!(path.exists(), "the lock file is created, parent and all");
    }

    #[test]
    fn real_path_is_under_application_support() {
        let path = settings_lock_path().expect("macOS always has a config dir");
        assert!(
            path.ends_with("Nudge/settings.lock"),
            "unexpected lock path: {}",
            path.display()
        );
        assert!(
            path.to_string_lossy().contains("Application Support"),
            "the lock belongs beside the app's other per-user state: {}",
            path.display()
        );
    }
}
