#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(target_arch = "wasm32"))]
mod app;
#[cfg(target_os = "windows")]
mod daisy;
#[cfg(not(target_arch = "wasm32"))]
mod journal;
#[cfg(not(target_arch = "wasm32"))]
mod nudge_state;
#[cfg(not(target_arch = "wasm32"))]
mod timer;
#[cfg(target_os = "windows")]
mod tray_bridge;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    use app::NudgeApp;
    use eframe::egui;

    // The tray icon, its menu, and the animation loop all live on a single
    // dedicated thread (see tray_bridge::spawn_tray_thread). It runs its
    // own message pump so animation keeps ticking even while eframe's
    // popup window is SW_HIDE'd.
    #[cfg(target_os = "windows")]
    tray_bridge::spawn_tray_thread();

    // Spotlight window: horizontally centered, vertical center at 40% of screen
    // per spec §1. Computed once at launch from primary monitor dimensions.
    let win_size = [520.0_f32, 320.0];
    #[cfg(target_os = "windows")]
    let viewport = {
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
        let (screen_w, screen_h) = unsafe {
            (
                GetSystemMetrics(SM_CXSCREEN).max(1) as u32,
                GetSystemMetrics(SM_CYSCREEN).max(1) as u32,
            )
        };
        let (x, y) =
            nudge_state::window_position((screen_w, screen_h), (win_size[0] as u32, win_size[1] as u32));
        egui::ViewportBuilder::default()
            .with_inner_size(win_size)
            .with_position([x as f32, y as f32])
            .with_decorations(false)
            .with_always_on_top()
            .with_resizable(false)
            .with_transparent(true)
            .with_taskbar(false)
            .with_title("Nudge")
    };
    #[cfg(not(target_os = "windows"))]
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size(win_size)
        .with_decorations(false)
        .with_always_on_top()
        .with_resizable(false)
        .with_title("Nudge");

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Nudge",
        options,
        Box::new(move |cc| Ok(Box::new(NudgeApp::new(cc)))),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {}
