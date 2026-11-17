use std::fs::{File, OpenOptions};
use std::io;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct SettingsLock {
    _file: Option<File>,
}

fn settings_lock_path() -> Option<PathBuf> {
    Some(dirs::config_dir()?.join("Nudge").join("settings.lock"))
}

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
