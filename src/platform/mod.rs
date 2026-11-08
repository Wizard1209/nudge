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
//! * **`macos`** — the interval timer, the menu-bar status item and the global
//!   hotkey, all polled from `App::logic` on the main thread, because AppKit
//!   and Carbon both insist on that thread.
//! * **fallback** — no-ops, so a Linux dev host still compiles and runs the
//!   popup (it just never nudges), which is the property the old
//!   `cfg(not(any(windows, macos)))` arms in `main.rs` / `autostart.rs` have.
//!
//! What the three have in common — the edge flags the app polls, the armed
//! timer, the parked sprite change — is written once, in [`shared`]; each
//! backend adds only a `wake` that `shared` calls after a write it must act
//! on, and the OS-shaped functions that are genuinely its own.
//!
//! Not compiled for wasm: the browser build has no OS integration at all, and
//! `std::time::Instant` — which `set_timer_state` takes — is not usable there.

pub(crate) mod shared;
pub use shared::{
    ArmedTimer, request_config_reload, set_timer_state, take_pending_sprite, take_timer_fired,
    take_tray_clicked,
};

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
