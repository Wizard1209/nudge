//! The seam between the app and whatever the OS needs to be told.
//!
//! `NudgeApp` and `main` call `crate::platform::…` and never cfg on the OS
//! themselves; each backend below exposes the same set of free functions and
//! decides what they mean here. The facade is deliberately shaped around what
//! the app needs — "reveal the popup", "a new deadline was set" — not around
//! any one OS's mechanism, so the two backends can be as different as they
//! actually are:
//!
//! * **`windows`** — a pure re-export of [`crate::tray_bridge`]. The tray
//!   thread there owns the timer, the icon animation and the popup wakeup, so
//!   the seam adds nothing: it only renames the two entry points whose
//!   signatures carried Win32 types across the boundary.
//! * **`macos`** — the interval timer, polled from `App::logic` on the main
//!   thread. No tray, no hotkey yet.
//! * **fallback** — no-ops, so a Linux dev host still compiles and runs the
//!   popup (it just never nudges), which is the property the old
//!   `cfg(not(any(windows, macos)))` arms in `main.rs` / `autostart.rs` have.
//!
//! Not compiled for wasm: the browser build has no OS integration at all, and
//! `std::time::Instant` — which `set_timer_state` takes — is not usable there.

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use self::windows::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use self::macos::*;

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
mod fallback;
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub use self::fallback::*;
