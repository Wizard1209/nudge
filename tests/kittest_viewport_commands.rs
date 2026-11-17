#![cfg(any(target_os = "macos", target_os = "windows"))]

use std::sync::{Mutex, MutexGuard};

use eframe::egui::{self, Rect, ViewportCommand, ViewportId, pos2, vec2};
use egui_kittest::Harness;

use nudge_web::app::NudgeApp;
use nudge_web::layout::ScreenGeometry;

fn geometry() -> ScreenGeometry {
    ScreenGeometry {
        monitor: Rect::from_min_size(pos2(0.0, 0.0), vec2(1470.0, 956.0)),
        work: Rect::from_min_size(pos2(0.0, 33.0), vec2(1470.0, 833.0)),
    }
}

fn one_frame(harness: &mut Harness<'_, NudgeApp>) -> Vec<ViewportCommand> {
    harness.step();
    harness
        .output()
        .viewport_output
        .get(&ViewportId::ROOT)
        .expect("kittest always reports the root viewport")
        .commands
        .clone()
}

static SEAM: Mutex<()> = Mutex::new(());

fn seam() -> MutexGuard<'static, ()> {
    SEAM.lock().unwrap_or_else(|e| e.into_inner())
}

fn harness() -> Harness<'static, NudgeApp> {
    Harness::builder()
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|cc| NudgeApp::new(cc, 10.0).with_screen_geometry(geometry()))
}

fn manual_open() {
    nudge_web::platform::request_manual_open();
}

fn hide() -> Vec<ViewportCommand> {
    if cfg!(target_os = "macos") {
        vec![ViewportCommand::InnerSize(vec2(1.0, 1.0))]
    } else {
        vec![ViewportCommand::Visible(false)]
    }
}

fn order_out() -> Vec<ViewportCommand> {
    if cfg!(target_os = "macos") {
        vec![ViewportCommand::Visible(false)]
    } else {
        vec![]
    }
}

fn reveal() -> Vec<ViewportCommand> {
    if cfg!(target_os = "macos") {
        let work = geometry().work;
        vec![
            ViewportCommand::InnerSize(work.size()),
            ViewportCommand::OuterPosition(work.min),
            ViewportCommand::Visible(true),
            ViewportCommand::Focus,
            ViewportCommand::Visible(true),
            ViewportCommand::Focus,
        ]
    } else {
        vec![ViewportCommand::Visible(true)]
    }
}

#[test]
fn hide_and_show_emit_exactly_these_viewport_commands() {
    let _seam = seam();
    let mut harness = harness();

    assert_eq!(one_frame(&mut harness), vec![], "a hidden frame is quiet");

    manual_open();
    assert_eq!(one_frame(&mut harness), reveal(), "first reveal frame");
    assert_eq!(one_frame(&mut harness), vec![], "a visible frame is quiet");

    harness.key_down(egui::Key::Escape);
    assert_eq!(one_frame(&mut harness), hide(), "Esc frame");
    harness.key_up(egui::Key::Escape);
    assert_eq!(one_frame(&mut harness), order_out(), "frame after the Esc");
    assert_eq!(one_frame(&mut harness), vec![], "a hidden frame is quiet");

    manual_open();
    assert_eq!(one_frame(&mut harness), reveal(), "manual reveal frame");
    assert_eq!(one_frame(&mut harness), vec![], "a visible frame is quiet");

    harness.key_down(egui::Key::Escape);
    assert_eq!(one_frame(&mut harness), hide(), "second Esc frame");
    harness.key_up(egui::Key::Escape);
    assert_eq!(
        one_frame(&mut harness),
        order_out(),
        "frame after the second Esc"
    );
    assert_eq!(one_frame(&mut harness), vec![], "a hidden frame is quiet");
}

#[test]
fn an_open_in_the_frame_after_a_close_stays_open() {
    let _seam = seam();
    let mut harness = harness();
    manual_open();
    assert_eq!(one_frame(&mut harness), reveal(), "first reveal frame");
    assert_eq!(one_frame(&mut harness), vec![], "a visible frame is quiet");

    harness.key_down(egui::Key::Escape);
    assert_eq!(one_frame(&mut harness), hide(), "Esc frame");
    harness.key_up(egui::Key::Escape);
    manual_open();
    assert_eq!(
        one_frame(&mut harness),
        reveal(),
        "the frame after the close"
    );

    for frame in 0..3 {
        assert_eq!(
            one_frame(&mut harness),
            vec![],
            "visible frame {frame} after the reveal is quiet"
        );
    }
}
