#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(target_arch = "wasm32"))]
mod app;
mod journal;
mod nudge_state;
mod timer;
#[cfg(target_os = "windows")]
mod tray_bridge;

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    use app::NudgeApp;
    use eframe::egui;

    // === Tray icon (Windows only) ===
    #[cfg(target_os = "windows")]
    let _tray_state = {
        use tray_icon::menu::{Menu, MenuItem};
        use tray_icon::TrayIconBuilder;

        let menu = Menu::new();
        let exit_item = MenuItem::new("Exit", true, None);
        menu.append(&exit_item).unwrap();

        // Simple 16x16 blue icon
        let icon_rgba = {
            let mut data = vec![0u8; 16 * 16 * 4];
            for pixel in data.chunks_exact_mut(4) {
                pixel[0] = 70;
                pixel[1] = 130;
                pixel[2] = 230;
                pixel[3] = 255;
            }
            data
        };
        let icon =
            tray_icon::Icon::from_rgba(icon_rgba, 16, 16).expect("failed to create tray icon");

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip("Nudge")
            .with_icon(icon)
            .build()
            .expect("failed to build tray icon");

        // Set up event handlers that run on Windows message thread.
        // These work even when window is SW_HIDE'd (unlike polling in update()).
        tray_icon::TrayIconEvent::set_event_handler(Some(|event| {
            // Only respond to left-click release (right-click opens context menu)
            if matches!(
                event,
                tray_icon::TrayIconEvent::Click {
                    button: tray_icon::MouseButton::Left,
                    button_state: tray_icon::MouseButtonState::Up,
                    ..
                }
            ) {
                tray_bridge::set_tray_clicked();
                // Restore window to wake up the eframe event loop
                if let Some(hwnd_val) = tray_bridge::load_hwnd() {
                    unsafe {
                        use windows::Win32::Foundation::HWND;
                        use windows::Win32::UI::WindowsAndMessaging::*;
                        let h = HWND(hwnd_val as *mut _);
                        let _ = ShowWindow(h, SW_RESTORE);
                        let _ = SetForegroundWindow(h);
                    }
                }
            }
        }));

        tray_icon::menu::MenuEvent::set_event_handler(Some(|_event| {
            std::process::exit(0);
        }));

        // Keep tray alive for the lifetime of the app
        (tray, exit_item)
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([600.0, 220.0])
            .with_decorations(false)
            .with_always_on_top()
            .with_resizable(false)
            .with_title("Nudge"),
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
