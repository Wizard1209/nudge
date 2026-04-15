#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(target_arch = "wasm32"))]
mod app;
#[cfg(not(target_arch = "wasm32"))]
mod journal;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    use app::NudgeApp;
    use eframe::egui;

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([600.0, 220.0])
            .with_decorations(false)
            .with_always_on_top()
            .with_resizable(false)
            .with_title("Nudge"),
        // TODO: center on screen (needs platform-specific logic)
        ..Default::default()
    };

    eframe::run_native("Nudge", options, Box::new(|cc| Ok(Box::new(NudgeApp::new(cc)))))
}

#[cfg(target_arch = "wasm32")]
fn main() {}
