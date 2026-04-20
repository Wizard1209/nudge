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
        let [show_label, quit_label] = nudge_state::TRAY_MENU_LABELS;
        let show_item = MenuItem::new(show_label, true, None);
        let quit_item = MenuItem::new(quit_label, true, None);
        menu.append(&show_item).unwrap();
        menu.append(&quit_item).unwrap();
        let show_id = show_item.id().clone();
        let quit_id = quit_item.id().clone();

        // Monochrome circle tray icon (placeholder — spec §5 asks for a
        // Gestimer-style stylized timer; pending an SVG asset).
        let icon_rgba = {
            let size = 16usize;
            let center = (size as f32 - 1.0) / 2.0;
            let mut data = vec![0u8; size * size * 4];
            for y in 0..size {
                for x in 0..size {
                    let dx = x as f32 - center;
                    let dy = y as f32 - center;
                    let r = (dx * dx + dy * dy).sqrt();
                    let i = (y * size + x) * 4;
                    let (rgba) = if r < center - 0.5 {
                        [230, 230, 230, 255]
                    } else {
                        [0, 0, 0, 0]
                    };
                    data[i..i + 4].copy_from_slice(&rgba);
                }
            }
            data
        };
        let icon =
            tray_icon::Icon::from_rgba(icon_rgba, 16, 16).expect("failed to create tray icon");

        let initial_tooltip = nudge_state::tooltip_for_remaining(std::time::Duration::from_secs(600));

        let tray = TrayIconBuilder::new()
            .with_menu(Box::new(menu))
            .with_menu_on_left_click(false)
            .with_tooltip(&initial_tooltip)
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

        tray_icon::menu::MenuEvent::set_event_handler(Some(move |event: tray_icon::menu::MenuEvent| {
            if event.id == quit_id {
                std::process::exit(0);
            }
            if event.id == show_id {
                tray_bridge::set_tray_clicked();
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

        // Keep tray alive for the lifetime of the app
        (tray, show_item, quit_item)
    };

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
