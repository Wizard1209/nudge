use eframe::egui;

const CELL: f32 = 8.0;

const DARK: egui::Color32 = egui::Color32::from_rgb(20, 20, 20);
const LIGHT: egui::Color32 = egui::Color32::from_rgb(200, 200, 200);

fn main() -> eframe::Result<()> {
    let pattern = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "checker".to_string());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            // Screen-filling, but deliberately NOT `with_fullscreen`: native
            // fullscreen on macOS moves the window into a Space of its own, so
            // the backdrop and the app under test end up on different desktops
            // and the capture silently photographs neither. The size is
            // applied on the first frame instead, once the monitor is known.
            .with_decorations(false)
            .with_window_level(egui::WindowLevel::AlwaysOnTop)
            .with_active(false),
        ..Default::default()
    };

    let (mut moved, mut sized_to) = (false, None::<egui::Vec2>);
    eframe::run_ui_native("nudge control backdrop", options, move |ui, _frame| {
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }

        // Fill the main display: move to its top-left, then size to it. Two
        // steps over several frames rather than one: the first frame runs
        // before the window has been moved, so the monitor it reports is
        // whichever one it happened to open on — with a second display that
        // is routinely not the main one the app under test picks — and the
        // move takes a frame to land. Screen origin (0,0) is the main
        // display's top-left by definition, so the size read after the move
        // is the right one.
        //
        // Each command is sent once per target, never re-sent until the window
        // matches: macOS keeps a window below the menu bar, so its position
        // never reads (0,0), and waiting for that would repaint forever and
        // never reach the resize.
        let monitor = ui.ctx().input(|i| i.viewport().monitor_size);
        if !moved {
            moved = true;
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(0.0, 0.0)));
            ui.ctx().request_repaint();
        } else if let Some(monitor) = monitor
            && sized_to != Some(monitor)
        {
            sized_to = Some(monitor);
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::InnerSize(monitor));
            ui.ctx().request_repaint();
        }
        let rect = ui.max_rect();
        let painter = ui.painter();
        painter.rect_filled(rect, 0, if pattern == "light" { LIGHT } else { DARK });
        if pattern != "checker" {
            return;
        }
        let cols = (rect.width() / CELL).ceil() as i32;
        let rows = (rect.height() / CELL).ceil() as i32;
        for row in 0..rows {
            for col in 0..cols {
                if (row + col) % 2 == 0 {
                    continue;
                }
                let min = rect.min + egui::vec2(col as f32 * CELL, row as f32 * CELL);
                painter.rect_filled(
                    egui::Rect::from_min_size(min, egui::vec2(CELL, CELL)).intersect(rect),
                    0,
                    LIGHT,
                );
            }
        }
    })
}
