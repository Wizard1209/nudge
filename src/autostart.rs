//! Launch-at-login autostart.
//!
//! Two layers, deliberately split:
//!
//! - [`AutostartProvider`] is the OS mechanism that actually makes the app
//!   start at login. Windows uses the `HKCU\…\Run` registry value; other
//!   platforms can grow their own impls (launch agents, `.desktop`, …). It's a
//!   trait so tests and the WASM build substitute an in-memory fake.
//!
//! - [`apply_autostart`] is the *transactional* rule on top: perform the OS
//!   change, confirm it through the system, and only then write `autostart`
//!   into the config. config.json therefore only ever claims a state the OS
//!   actually has — a registry write that silently fails never leaves a
//!   `true` lie in the file. This is what lets the rest of the app treat the
//!   config as the source of truth.

use crate::config::{Config, ConfigError};

/// The OS mechanism for launch-at-login. Errors carry a human-readable detail
/// string (the underlying registry / OS error) for surfacing to the user.
pub trait AutostartProvider {
    /// Register the app to launch at login.
    fn enable(&self) -> Result<(), String>;
    /// Unregister. Disabling something already disabled must succeed (the
    /// post-condition "not registered" already holds).
    fn disable(&self) -> Result<(), String>;
    /// Whether the app is currently registered. Used both to drive the UI
    /// toggle and to *confirm* an `enable`/`disable` actually took effect.
    fn is_enabled(&self) -> Result<bool, String>;
}

/// Why an [`apply_autostart`] call failed. Split so the caller can tell
/// "the OS refused" (nothing changed, config untouched) from "the OS change
/// stuck but persisting the config failed" (system and config now disagree on
/// disk until the next save).
#[derive(Debug)]
pub enum AutostartError {
    /// The provider call failed, or the system didn't confirm the change.
    /// Config was left untouched.
    Backend(String),
    /// The OS change succeeded and was confirmed, but writing the config
    /// back failed.
    Persist(ConfigError),
}

impl std::fmt::Display for AutostartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AutostartError::Backend(d) => write!(f, "autostart: {d}"),
            AutostartError::Persist(e) => write!(f, "autostart persisted change failed: {e}"),
        }
    }
}

impl std::error::Error for AutostartError {}

/// Transactionally move autostart to `desired`.
///
/// Order is load-bearing: OS change → confirm via `is_enabled` → mutate
/// `config` → persist. If the OS change fails or isn't confirmed, `config` is
/// left exactly as it was and `persist` is never called — so a failed registry
/// write can't produce a config that claims autostart is on. Persistence is
/// injected as a closure so this stays platform-agnostic and unit-testable
/// (native passes `config::save`, WASM passes `config::save_to_localstorage`).
pub fn apply_autostart<F>(
    provider: &dyn AutostartProvider,
    config: &mut Config,
    desired: bool,
    persist: F,
) -> Result<(), AutostartError>
where
    F: FnOnce(&Config) -> Result<(), ConfigError>,
{
    let os_result = if desired {
        provider.enable()
    } else {
        provider.disable()
    };
    os_result.map_err(AutostartError::Backend)?;

    let confirmed = provider.is_enabled().map_err(AutostartError::Backend)?;
    if confirmed != desired {
        return Err(AutostartError::Backend(format!(
            "system did not confirm autostart={desired} (is_enabled reports {confirmed})"
        )));
    }

    config.autostart = desired;
    persist(config).map_err(AutostartError::Persist)
}

/// In-memory provider for tests and the WASM build (no real registry in the
/// browser). Tracks the enabled flag and can be told to fail a given call.
/// Deliberately absent from the native release binary, which only ever uses
/// the real OS provider.
#[cfg(any(test, target_arch = "wasm32"))]
pub struct FakeProvider {
    enabled: std::cell::Cell<bool>,
    fail_enable: bool,
    fail_disable: bool,
    /// When true, `is_enabled` reports the *opposite* of the tracked state —
    /// simulates an OS change that silently didn't stick.
    lie_on_confirm: bool,
}

#[cfg(any(test, target_arch = "wasm32"))]
impl FakeProvider {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled: std::cell::Cell::new(enabled),
            fail_enable: false,
            fail_disable: false,
            lie_on_confirm: false,
        }
    }
    pub fn failing_enable() -> Self {
        Self {
            fail_enable: true,
            ..Self::new(false)
        }
    }
    pub fn failing_disable() -> Self {
        Self {
            fail_disable: true,
            ..Self::new(true)
        }
    }
    pub fn unconfirmed() -> Self {
        Self {
            lie_on_confirm: true,
            ..Self::new(false)
        }
    }
}

#[cfg(any(test, target_arch = "wasm32"))]
impl AutostartProvider for FakeProvider {
    fn enable(&self) -> Result<(), String> {
        if self.fail_enable {
            return Err("fake enable failed".to_string());
        }
        self.enabled.set(true);
        Ok(())
    }
    fn disable(&self) -> Result<(), String> {
        if self.fail_disable {
            return Err("fake disable failed".to_string());
        }
        self.enabled.set(false);
        Ok(())
    }
    fn is_enabled(&self) -> Result<bool, String> {
        Ok(self.enabled.get() ^ self.lie_on_confirm)
    }
}

/// Registry-name the app registers itself under in the `Run` key. Stable
/// identity so enable/disable/is_enabled all address the same value.
#[cfg(target_os = "windows")]
pub const RUN_VALUE_NAME: &str = "Nudge";

/// The per-user autostart key Windows reads at login.
#[cfg(target_os = "windows")]
pub const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Windows autostart via the per-user `HKCU\…\Run` registry value. Storing the
/// value makes Windows launch `<exe>` at login; deleting it stops that.
/// `is_enabled` checks only for the value's presence (not that it still points
/// at the current exe), which is enough to drive the toggle.
#[cfg(target_os = "windows")]
pub struct WindowsRegistryProvider {
    /// Command line written into the Run value. Defaults to the current exe.
    exe: std::path::PathBuf,
}

#[cfg(target_os = "windows")]
impl WindowsRegistryProvider {
    /// Provider targeting the currently running executable.
    pub fn for_current_exe() -> Result<Self, String> {
        let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
        Ok(Self { exe })
    }
}

#[cfg(target_os = "windows")]
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(target_os = "windows")]
impl AutostartProvider for WindowsRegistryProvider {
    fn enable(&self) -> Result<(), String> {
        use windows::Win32::Foundation::ERROR_SUCCESS;
        use windows::Win32::System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegOpenKeyExW,
            RegSetValueExW,
        };
        use windows::core::PCWSTR;

        let subkey = to_wide(RUN_SUBKEY);
        let name = to_wide(RUN_VALUE_NAME);
        // The Run value is a command line; quote the path so spaces survive.
        let data = to_wide(&format!("\"{}\"", self.exe.display()));
        let data_bytes = unsafe {
            std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(&data[..]))
        };

        unsafe {
            let mut hkey = HKEY::default();
            let rc = RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                Some(0),
                KEY_SET_VALUE,
                &mut hkey,
            );
            if rc != ERROR_SUCCESS {
                return Err(format!("RegOpenKeyExW(Run): {rc:?}"));
            }
            let set = RegSetValueExW(hkey, PCWSTR(name.as_ptr()), Some(0), REG_SZ, Some(data_bytes));
            let _ = RegCloseKey(hkey);
            if set != ERROR_SUCCESS {
                return Err(format!("RegSetValueExW(Nudge): {set:?}"));
            }
        }
        Ok(())
    }

    fn disable(&self) -> Result<(), String> {
        use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
        use windows::Win32::System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, RegCloseKey, RegDeleteValueW, RegOpenKeyExW,
        };
        use windows::core::PCWSTR;

        let subkey = to_wide(RUN_SUBKEY);
        let name = to_wide(RUN_VALUE_NAME);

        unsafe {
            let mut hkey = HKEY::default();
            let rc = RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                Some(0),
                KEY_SET_VALUE,
                &mut hkey,
            );
            if rc != ERROR_SUCCESS {
                return Err(format!("RegOpenKeyExW(Run): {rc:?}"));
            }
            let del = RegDeleteValueW(hkey, PCWSTR(name.as_ptr()));
            let _ = RegCloseKey(hkey);
            // Deleting an already-absent value is a no-op success: the
            // post-condition "not registered" already holds.
            if del != ERROR_SUCCESS && del != ERROR_FILE_NOT_FOUND {
                return Err(format!("RegDeleteValueW(Nudge): {del:?}"));
            }
        }
        Ok(())
    }

    fn is_enabled(&self) -> Result<bool, String> {
        use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
        use windows::Win32::System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
        };
        use windows::core::PCWSTR;

        let subkey = to_wide(RUN_SUBKEY);
        let name = to_wide(RUN_VALUE_NAME);

        unsafe {
            let mut hkey = HKEY::default();
            let rc = RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                Some(0),
                KEY_QUERY_VALUE,
                &mut hkey,
            );
            if rc != ERROR_SUCCESS {
                return Err(format!("RegOpenKeyExW(Run): {rc:?}"));
            }
            // Querying with null data buffers just probes existence.
            let q = RegQueryValueExW(hkey, PCWSTR(name.as_ptr()), None, None, None, None);
            let _ = RegCloseKey(hkey);
            if q == ERROR_SUCCESS {
                Ok(true)
            } else if q == ERROR_FILE_NOT_FOUND {
                Ok(false)
            } else {
                Err(format!("RegQueryValueExW(Nudge): {q:?}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recording_persist(sink: &std::cell::RefCell<Option<Config>>) -> impl FnOnce(&Config) -> Result<(), ConfigError> + '_ {
        move |c: &Config| {
            *sink.borrow_mut() = Some(c.clone());
            Ok(())
        }
    }

    #[test]
    fn enable_success_sets_and_persists_config() {
        let provider = FakeProvider::new(false);
        let mut config = Config::default();
        let saved = std::cell::RefCell::new(None);

        apply_autostart(&provider, &mut config, true, recording_persist(&saved)).unwrap();

        assert!(config.autostart, "in-memory config reflects the change");
        assert!(provider.is_enabled().unwrap(), "OS state actually enabled");
        assert_eq!(
            saved.borrow().as_ref().map(|c| c.autostart),
            Some(true),
            "persisted config carries autostart=true"
        );
    }

    #[test]
    fn disable_success_clears_and_persists_config() {
        let provider = FakeProvider::new(true);
        let mut config = Config {
            autostart: true,
            ..Config::default()
        };
        let saved = std::cell::RefCell::new(None);

        apply_autostart(&provider, &mut config, false, recording_persist(&saved)).unwrap();

        assert!(!config.autostart);
        assert!(!provider.is_enabled().unwrap());
        assert_eq!(saved.borrow().as_ref().map(|c| c.autostart), Some(false));
    }

    #[test]
    fn backend_failure_leaves_config_untouched_and_skips_persist() {
        // The registry write fails: config must NOT flip, and persist must
        // never run — otherwise config.json would claim a state the OS
        // doesn't have.
        let provider = FakeProvider::failing_enable();
        let mut config = Config::default();
        let saved = std::cell::RefCell::new(None);

        let err = apply_autostart(&provider, &mut config, true, recording_persist(&saved))
            .unwrap_err();

        assert!(matches!(err, AutostartError::Backend(_)));
        assert!(!config.autostart, "config not mutated on backend failure");
        assert!(saved.borrow().is_none(), "persist never called on backend failure");
    }

    #[test]
    fn unconfirmed_change_is_a_backend_error() {
        // enable() returns Ok but is_enabled disagrees → we must not trust it.
        let provider = FakeProvider::unconfirmed();
        let mut config = Config::default();
        let saved = std::cell::RefCell::new(None);

        let err = apply_autostart(&provider, &mut config, true, recording_persist(&saved))
            .unwrap_err();

        assert!(matches!(err, AutostartError::Backend(_)));
        assert!(!config.autostart);
        assert!(saved.borrow().is_none());
    }

    #[test]
    fn persist_failure_surfaces_as_persist_error() {
        // OS change stuck and confirmed, but the disk write failed.
        let provider = FakeProvider::new(false);
        let mut config = Config::default();

        let err = apply_autostart(&provider, &mut config, true, |_c| {
            Err(ConfigError::Io {
                path: "x".to_string(),
                detail: "disk full".to_string(),
            })
        })
        .unwrap_err();

        assert!(matches!(err, AutostartError::Persist(_)));
        assert!(provider.is_enabled().unwrap(), "OS change still happened");
    }

    #[test]
    fn enable_is_idempotent() {
        let provider = FakeProvider::new(true); // already enabled
        let mut config = Config::default();
        let saved = std::cell::RefCell::new(None);

        apply_autostart(&provider, &mut config, true, recording_persist(&saved)).unwrap();

        assert!(config.autostart);
        assert_eq!(saved.borrow().as_ref().map(|c| c.autostart), Some(true));
    }
}
