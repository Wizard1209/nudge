//! What the app tells the window on hide and on show, headless.
//!
//! Every other kittest file asserts on the card — its fields, its layout —
//! and none looks at the `ViewportCommand`s a frame emits. Those are what
//! actually order the native window in and out and size it, so a hide that
//! stopped hiding, or a show that stopped placing, would leave the card tests
//! green. This file pins the commands of an `Esc` and of a manual reveal, one
//! frame at a time. The reveal is raised where the OS raises it, so on macOS
//! it is `App::logic` that orders the window in, as it is for the timer.
//!
//! The expected lists are exact on purpose: a change to how the popup is
//! hidden or revealed — releasing the window instead of ordering it out,
//! parking it at another size, re-placing it in another order — has to
//! update them here, deliberately, rather than slip past.
//!
//! Its own test binary, like `macos_manual_open.rs`: the reveal goes through
//! the platform seam's process-global flags, and the tests here take [`SEAM`]
//! for the same reason.

#![cfg(any(target_os = "macos", target_os = "windows"))]

use std::sync::{Mutex, MutexGuard};

use eframe::egui::{self, Rect, ViewportCommand, ViewportId, pos2, vec2};
use egui_kittest::Harness;

use nudge_web::app::NudgeApp;
use nudge_web::sprite::ScreenGeometry;

/// A 1470×956 display with a 33 pt menu bar.
fn geometry() -> ScreenGeometry {
    ScreenGeometry {
        monitor: Rect::from_min_size(pos2(0.0, 0.0), vec2(1470.0, 956.0)),
        work: Rect::from_min_size(pos2(0.0, 33.0), vec2(1470.0, 833.0)),
    }
}

/// Run exactly one frame — the harness runs one per queued event, so callers
/// queue at most one — and return the root viewport's commands from it.
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

/// Held for a whole test: the platform seam's flags are per process.
static SEAM: Mutex<()> = Mutex::new(());

fn seam() -> MutexGuard<'static, ()> {
    SEAM.lock().unwrap_or_else(|e| e.into_inner())
}

fn harness() -> Harness<'static, NudgeApp> {
    // Ten minutes, so the interval timer cannot emit anything here.
    Harness::builder()
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|cc| NudgeApp::new(cc, 10.0).with_screen_geometry(geometry()))
}

/// A status-item click or the hotkey, as the OS delivers it. macOS: the
/// flag their handlers raise, which `tick` turns into a reveal in
/// `App::logic`. Windows: the edge the tray thread raises after it has shown
/// the window itself.
fn manual_open() {
    #[cfg(target_os = "macos")]
    nudge_web::platform::request_manual_open();
    #[cfg(target_os = "windows")]
    nudge_web::platform::set_tray_clicked();
}

fn then(mut a: Vec<ViewportCommand>, b: Vec<ViewportCommand>) -> Vec<ViewportCommand> {
    a.extend(b);
    a
}

/// The hide: an order-out, and on macOS the shrink that parks the hidden
/// window (`NudgeApp::hide_popup`). Windows keeps the window's size.
fn hide() -> Vec<ViewportCommand> {
    let park = if cfg!(target_os = "macos") {
        vec![ViewportCommand::InnerSize(vec2(1.0, 1.0))]
    } else {
        vec![]
    };
    then(vec![ViewportCommand::Visible(false)], park)
}

/// The frame a manual open lands on.
///
/// macOS: `App::logic` gives the parked window its work-area frame back,
/// orders it in and asks for the keyboard; then `ui()` makes the
/// `show_popup` transition, which repeats the order-in and the focus request.
/// eframe runs that `ui()` one frame later, once the window is visible; the
/// harness runs it in the same pass.
///
/// Windows: the tray thread has already shown the HWND, nothing the window
/// was placed for has changed, and `show_popup` only tells egui.
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

    // The launch frame — placement plus the launch show — is not observable:
    // the builder runs it and then `run_ok`s to a stable frame before handing
    // the harness over, and `output()` keeps only the last frame's commands.
    // What is left is the launch popup, up and placed.
    assert_eq!(one_frame(&mut harness), vec![], "a visible frame is quiet");

    // Esc: the hide orders the window out — and on macOS parks it.
    harness.key_down(egui::Key::Escape);
    assert_eq!(one_frame(&mut harness), hide(), "Esc frame");
    harness.key_up(egui::Key::Escape);
    assert_eq!(one_frame(&mut harness), vec![], "a hidden frame is quiet");

    manual_open();
    assert_eq!(one_frame(&mut harness), reveal(), "manual reveal frame");
    assert_eq!(one_frame(&mut harness), vec![], "a visible frame is quiet");

    harness.key_down(egui::Key::Escape);
    assert_eq!(one_frame(&mut harness), hide(), "second Esc frame");
}

/// An open raised between the close's frame and the next one: the reveal is
/// whole, and nothing left over from the hide orders the window out again.
#[test]
fn an_open_in_the_frame_after_a_close_stays_open() {
    let _seam = seam();
    let mut harness = harness();
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
