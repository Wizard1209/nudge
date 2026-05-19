#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(target_arch = "wasm32"))]
mod app;
#[cfg(not(target_arch = "wasm32"))]
mod config;
#[cfg(target_os = "windows")]
mod daisy;
mod hotkey;
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

    // Load the user config (or use defaults). Bad / missing files are
    // logged but never fatal — the app must always come up.
    let config_path = config::resolve_default_config_path();
    let (config, config_err) = config::load_or_default(&config_path);
    if let Some(err) = config_err {
        eprintln!("[nudge] {err}");
    } else if !config_path.exists() {
        // First run: seed the file so users have a template to edit.
        if let Err(e) = config::ensure_default_written(&config_path) {
            eprintln!("[nudge] failed to write default config: {e}");
        }
    }
    let (configured_hotkey, hotkey_was_invalid) = config.resolved_hotkey();
    if hotkey_was_invalid {
        eprintln!(
            "[nudge] config hotkey \"{}\" is unparseable, falling back to default",
            config.hotkey
        );
    }

    // The tray icon, its menu, and the animation loop all live on a single
    // dedicated thread (see tray_bridge::spawn_tray_thread). It runs its
    // own message pump so animation keeps ticking even while eframe's
    // popup window is SW_HIDE'd. The same thread also owns the global
    // hotkey registration (WM_HOTKEY is delivered to the registering
    // thread's message queue).
    #[cfg(target_os = "windows")]
    tray_bridge::spawn_tray_thread(Some(configured_hotkey));
    #[cfg(not(target_os = "windows"))]
    let _ = configured_hotkey; // unused on non-Windows targets for now

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
