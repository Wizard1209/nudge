//! Windows backend: `tray_bridge`, verbatim.
//!
//! This is the shipped product, and the port cannot test it, so nothing in
//! here may add or reorder behaviour — every function is either a re-export
//! or a one-line adapter around the identical `tray_bridge` call the app made
//! before the seam existed.
//!
//! Only two entry points needed adapting, both because their signatures
//! carried Win32 types (`HWND`) that must not cross the seam. The adaptation
//! lives here rather than in `tray_bridge` so the tray module keeps its own
//! shape.

use eframe::egui;

use crate::config;
use crate::hotkey::Hotkey;

pub use crate::tray_bridge::{
    request_config_reload, set_popup_visible, set_timer_state, take_timer_fired, take_tray_clicked,
};

/// The raise side of `take_tray_clicked`. No caller goes through the seam for
/// it today — on Windows the tray thread's own event handlers set the flag
/// directly, and macOS has no status item yet — but it is half of a pair, and
/// dropping it would leave the macOS tray with nothing to call when it lands.
#[allow(unused_imports)]
pub use crate::tray_bridge::set_tray_clicked;

/// Start the OS-integration machinery: here, the tray thread, which also
/// owns the global hotkey and the config-reload diff.
pub fn start(hotkey: Option<Hotkey>, config_path: std::path::PathBuf, initial: config::Config) {
    crate::tray_bridge::spawn_tray_thread(hotkey, config_path, initial);
}

/// No-op: `start` already built everything on its own thread, and none of it
/// needs the event loop. The macOS arm is the one with work here — its status
/// item and hotkey cannot exist until the main thread is the event loop's.
pub fn start_on_event_loop(_ctx: &egui::Context) {}

/// `tray_bridge::store_hwnd` under a name that doesn't presume a handle.
pub fn note_window_handle(handle: isize) {
    crate::tray_bridge::store_hwnd(handle);
}

/// Make the popup the foreground window. Reads the stored handle itself so
/// `HWND` stays inside the Windows arm; `ctx` is unused because Win32 does
/// the whole job here (macOS needs a viewport command as well).
pub fn focus_popup(_ctx: &egui::Context) {
    if let Some(hwnd) = crate::tray_bridge::load_hwnd() {
        crate::tray_bridge::force_foreground(::windows::Win32::Foundation::HWND(hwnd as *mut _));
    }
}

/// No-op: the tray thread already polls the deadline on its own message
/// pump and `ShowWindow`s the popup when it expires.
pub fn tick(_ctx: &egui::Context) {}

/// No-op: the frosted glass behind the card (spec §2) is a macOS
/// `NSVisualEffectView`. The Windows counterpart would be a DWM backdrop, and
/// it is not implemented — the card there is a plain translucent fill, as it
/// has always been.
pub fn sync_card_backdrop(_ctx: &egui::Context, _frame: &eframe::Frame, _card: egui::Rect) {}
