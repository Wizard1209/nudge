//! A full-screen checkerboard to put *behind* the popup while a native test
//! photographs it.
//!
//! Why a controlled background is needed at all: every defect of a
//! see-through surface is differential against what is behind it. Over a
//! smooth desktop, "the card blurs its backdrop", "the card merely dims it"
//! and "the card is opaque" all produce the same picture, so a capture taken
//! over the developer's wallpaper cannot fail for those causes and carries no
//! evidence about them.
//!
//! A checkerboard separates them, because alpha and blur act on different
//! things. Alpha scales the checker's contrast but leaves its edges where they
//! are; a blur destroys the edges and collapses the squares toward their mean.
//! So the discriminating measurement is high-spatial-frequency energy behind
//! the card versus beside it — see `card_is_frosted_glass_not_a_tinted_pane` in
//! `tests/macos_native.rs`. Cells are deliberately small relative to the
//! card's blur radius: a checker coarser than the radius survives blurring and
//! would make a correct render look like a failure.
//!
//! Two patterns, because one is not enough to pin the surface down. The
//! checkerboard shows whether the backdrop's *detail* survives; running again
//! against a flat dark field and comparing the card's tone shows whether the
//! backdrop reaches it at all. Without the second, an opaque card passes the
//! first trivially — it too destroys every trace of the checker.
//!
//! A third pattern, `light`, is a flat bright field. It exists for measuring
//! what the app draws *outside* the card — a shadow's falloff has no signal
//! against a checkerboard, whose own cell edges dwarf it, nor against a dark
//! field, which a dark shadow cannot darken further.
//!
//! Run: `cargo run --example control_backdrop [checker|dark|light]` (Esc or SIGTERM
//! to quit; `checker` is the default). It takes no focus, so launching it
//! before the app under test does not trigger the popup's focus-loss
//! dismissal (spec §4).
//!
//! The window paints its first frame at the default size and only then
//! resizes to the monitor, and anything the OS puts above it (a system alert)
//! is in the photograph too, so a caller must *verify* the pattern is what it
//! sampled rather than trust that it is up — see the `BACKDROP_*_LUMA` checks
//! in `tests/macos_native.rs`.

use eframe::egui;

/// Checker cell, in points. Must stay well under the card's blur radius.
const CELL: f32 = 8.0;

/// The checker's two tones. Each doubles as a whole flat field — `dark` and
/// `light` — so a run differs from the checker run only in spatial frequency,
/// or from the other flat run only in tone.
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
            // Above ordinary windows, so the test photographs the card over
            // *this* and not over whatever the developer left open. Still
            // below the popup: that is always-on-top too and opens later, so
            // it wins the tie within the level.
            .with_window_level(egui::WindowLevel::AlwaysOnTop)
            // Never steal the keyboard: the app under test hides its popup on
            // focus loss, so a backdrop that activates itself would dismiss
            // the very window the test is about to photograph.
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
