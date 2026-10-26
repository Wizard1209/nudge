//! Backend for every other target — in practice a Linux dev host.
//!
//! All no-ops. The property being preserved is the one the old
//! `cfg(not(any(windows, macos)))` arms in `main.rs` and `autostart.rs`
//! already had: the tree builds and the popup runs on a machine that is
//! neither of the two shipping platforms, so a developer there can work on
//! the portable half. It never nudges, and never did.
//!
//! Blanket `dead_code` allow because that is precisely what this file is:
//! the whole facade, present so the seam type-checks, with no caller for the
//! parts of it this target never reaches. Do not copy the attribute into a
//! backend with real behaviour — there an unused function means something.

#![allow(dead_code)]

use std::time::{Duration, Instant};

use eframe::egui;

use crate::config;
use crate::hotkey::Hotkey;
use crate::sprite::ScreenGeometry;

pub fn start(_hotkey: Option<Hotkey>, _config_path: std::path::PathBuf, _initial: config::Config) {}

pub fn start_on_event_loop(_ctx: &egui::Context) {}

pub fn note_window_handle(_handle: isize) {}

pub fn set_tray_clicked() {}

pub fn take_tray_clicked() -> bool {
    false
}

pub fn take_timer_fired() -> bool {
    false
}

pub fn set_timer_state(_deadline: Instant, _duration: Duration) {}

pub fn set_popup_visible(_visible: bool) {}

pub fn request_config_reload() {}

pub fn take_pending_sprite() -> Option<config::SpriteConfig> {
    None
}

pub fn focus_popup(_ctx: &egui::Context) {}

pub fn tick(_ctx: &egui::Context) {}

pub fn sync_card_backdrop(_ctx: &egui::Context, _frame: &eframe::Frame, _card: egui::Rect) {}

pub fn screen_geometry(_ctx: &egui::Context, _frame: &eframe::Frame) -> Option<ScreenGeometry> {
    None
}

pub fn install_native_fonts(_ctx: &egui::Context) {}

/// No key source here, so the settings window's hotkey row is the plain typed
/// field it has always been. `None` is the whole answer: the row asks for a
/// source and renders a Record button only if it gets one.
pub fn new_edge_source(_ctx: &egui::Context) -> Option<Box<dyn crate::recorder::EdgeSource>> {
    None
}

pub fn hotkey_conflict(_hk: &Hotkey) -> Option<String> {
    None
}
