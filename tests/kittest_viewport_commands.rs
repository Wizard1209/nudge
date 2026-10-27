//! What the app tells the window on hide and on show, headless.
//!
//! Every other kittest file asserts on the card — its fields, its layout —
//! and none looks at the `ViewportCommand`s a frame emits. Those are what
//! actually order the native window in and out and size it, so a hide that
//! stopped hiding, or a show that stopped placing, would leave the card tests
//! green. This file pins the commands of an `Esc` and of a manual reveal, one
//! frame at a time.
//!
//! The expected lists are exact on purpose: a change to how the popup is
//! hidden or revealed — releasing the window instead of ordering it out,
//! shrinking it while hidden, re-placing it on every show — has to update
//! them here, deliberately, rather than slip past.
//!
//! Its own test binary, like `macos_manual_open.rs`: the reveal goes through
//! the platform seam's process-global tray flag.

#![cfg(any(target_os = "macos", target_os = "windows"))]

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

/// What `show_popup` sends on top of `Visible(true)`: the activation request
/// on macOS (`platform::macos::focus_popup`); Win32 activates the HWND
/// directly and sends nothing.
fn focus() -> Vec<ViewportCommand> {
    if cfg!(target_os = "macos") {
        vec![ViewportCommand::Focus]
    } else {
        vec![]
    }
}

fn then(mut a: Vec<ViewportCommand>, b: Vec<ViewportCommand>) -> Vec<ViewportCommand> {
    a.extend(b);
    a
}

#[test]
fn hide_and_show_emit_exactly_these_viewport_commands() {
    // Ten minutes, so the interval timer cannot emit anything here.
    let mut harness = Harness::builder()
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|cc| NudgeApp::new(cc, 10.0).with_screen_geometry(geometry()));

    // The launch frame — placement plus the launch show — is not observable:
    // the builder runs it and then `run_ok`s to a stable frame before handing
    // the harness over, and `output()` keeps only the last frame's commands.
    // What is left is the launch popup, up and placed.
    assert_eq!(one_frame(&mut harness), vec![], "a visible frame is quiet");

    // Esc: the hide is an order-out of the same window, nothing else — it
    // keeps its work-area size while hidden.
    harness.key_down(egui::Key::Escape);
    assert_eq!(
        one_frame(&mut harness),
        vec![ViewportCommand::Visible(false)],
        "Esc frame"
    );
    harness.key_up(egui::Key::Escape);
    assert_eq!(one_frame(&mut harness), vec![], "a hidden frame is quiet");

    // A status-item click / hotkey: the reveal orders the window back in. No
    // placement, because nothing it was placed for has changed — the window
    // still has the size it was given at launch.
    nudge_web::platform::set_tray_clicked();
    assert_eq!(
        one_frame(&mut harness),
        then(vec![ViewportCommand::Visible(true)], focus()),
        "manual reveal frame"
    );

    harness.key_down(egui::Key::Escape);
    assert_eq!(
        one_frame(&mut harness),
        vec![ViewportCommand::Visible(false)],
        "second Esc frame"
    );
}
