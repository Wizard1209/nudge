#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(target_arch = "wasm32"))]
mod app;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    use app::NudgeApp;
    use eframe::egui;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([400.0, 200.0])
            .with_title("Nudge"),
        ..Default::default()
    };

    eframe::run_native("Nudge", options, Box::new(|cc| Ok(Box::new(NudgeApp::new(cc)))))
}

#[cfg(target_arch = "wasm32")]
fn main() {}
