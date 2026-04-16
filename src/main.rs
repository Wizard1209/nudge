#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(target_arch = "wasm32"))]
mod app;
mod journal;
mod nudge_state;
mod timer;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    use app::NudgeApp;
    use eframe::egui;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    let tray_show_flag = Arc::new(AtomicBool::new(false));
    let exit_flag = Arc::new(AtomicBool::new(false));

    // === Tray icon (Windows only) ===
    #[cfg(target_os = "windows")]
    let _tray = {
        use tray_icon::menu::{Menu, MenuEvent, MenuItem};
        use tray_icon::{TrayIconBuilder, TrayIconEvent};

        let menu = Menu::new();
        let exit_item = MenuItem::new("Exit", true, None);
        menu.append(&exit_item).unwrap();

        // Simple 16x16 blue icon
        let icon_rgba = {
            let mut data = vec![0u8; 16 * 16 * 4];
            for pixel in data.chunks_exact_mut(4) {
                pixel[0] = 70;  // R
                pixel[1] = 130; // G
                pixel[2] = 230; // B
                pixel[3] = 255; // A
            }
            data
        };
        let icon =
            tray_icon::Icon::from_rgba(icon_rgba, 16, 16).expect("failed to create tray icon");

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_tooltip("Nudge")
            .with_icon(icon)
            .build()
            .expect("failed to build tray icon");

        // Handle tray click
        let flag_for_click = tray_show_flag.clone();
        std::thread::spawn(move || loop {
            if let Ok(event) = TrayIconEvent::receiver().recv() {
                if matches!(event, TrayIconEvent::Click { .. }) {
                    flag_for_click.store(true, Ordering::Relaxed);
                }
            }
        });

        // Handle menu exit
        let exit_flag_for_menu = exit_flag.clone();
        let exit_id = exit_item.id().clone();
        std::thread::spawn(move || loop {
            if let Ok(event) = MenuEvent::receiver().recv() {
                if event.id() == &exit_id {
                    exit_flag_for_menu.store(true, Ordering::Relaxed);
                }
            }
        });

        tray // keep alive
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([600.0, 220.0])
            .with_decorations(false)
            .with_always_on_top()
            .with_resizable(false)
            .with_title("Nudge"),
        // TODO: center on screen
        ..Default::default()
    };

    let flag = tray_show_flag.clone();
    let exit = exit_flag.clone();
    eframe::run_native(
        "Nudge",
        options,
        Box::new(move |cc| Ok(Box::new(NudgeApp::new(cc, flag, exit)))),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {}
