use eframe::egui;

use crate::config;
use crate::hotkey::Hotkey;
use crate::layout::ScreenGeometry;

pub use super::shared::set_popup_visible;

pub(super) fn wake() {}

pub fn allow_evicted_file_downloads() {}

pub fn start(_hotkey: Option<Hotkey>, _config_path: std::path::PathBuf, _initial: config::Config) {}

pub fn start_on_event_loop(_ctx: &egui::Context) {}

pub fn adopt_window(_frame: &eframe::Frame) {}

pub fn popup_is_foreground(_ctx: &egui::Context) -> bool {
    false
}

pub fn request_manual_open() {}

pub fn focus_popup(_ctx: &egui::Context) {}

pub fn tick(_ctx: &egui::Context) -> bool {
    false
}

pub fn sync_card_backdrop(_ctx: &egui::Context, _frame: &eframe::Frame, _card: egui::Rect) {}

pub fn screen_geometry(_ctx: &egui::Context, _frame: &eframe::Frame) -> Option<ScreenGeometry> {
    None
}

pub fn install_native_fonts(_ctx: &egui::Context) {}

pub struct SettingsLock;

pub fn acquire_settings_singleton() -> Option<SettingsLock> {
    Some(SettingsLock)
}

pub fn new_edge_source(_ctx: &egui::Context) -> Option<Box<dyn crate::recorder::EdgeSource>> {
    None
}

pub fn hotkey_conflict(_hk: &Hotkey) -> Option<String> {
    None
}
