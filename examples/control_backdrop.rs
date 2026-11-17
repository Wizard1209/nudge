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
            // macOS native full screen puts the window into a Space of its own, apart from the
            // app under test, so the window is sized by hand instead of with `with_fullscreen`.
            // proof: https://developer.apple.com/library/archive/documentation/General/Conceptual/MOSXAppProgrammingGuide/FullScreenApp/FullScreenApp.html
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
