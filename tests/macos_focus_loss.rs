//! Spec §4's "Switch by window focus loss", headless.
//!
//! On macOS this was dead code: `NudgeApp::current_foreground_matches` had a
//! Win32 arm, a wasm arm, and a `false` arm that macOS fell into — so `armed`
//! was never true and the focus-loss branch was unreachable. One Cmd-Tab away
//! from an open card left it floating on top of whatever the user switched to,
//! no longer key (so Esc and Enter went to the other app) and with
//! `POPUP_VISIBLE` still true, which parks `platform::macos::tick` and stops
//! the nudger for good.
//!
//! The macOS arm reads `viewport().focused`, which egui-winit refreshes from
//! `NSWindow::isKeyWindow` at the top of every frame — so this tier can drive
//! it by writing the same field into the harness's `RawInput`, and the whole
//! arm/close decision is exercised without a window server.
//!
//! What it cannot cover, and what a live session is still for: that AppKit
//! actually makes the revealed window key after `activateIgnoringOtherApps`
//! under `ActivationPolicy::Accessory`, and how many frames that takes. The
//! latch under test is what makes the answer to "how many" not matter.
//!
//! Its own test binary for the same reason `macos_timer.rs` and
//! `macos_manual_open.rs` are: the platform seam keeps the popup-visible flag
//! and the deadline in process globals.

#![cfg(target_os = "macos")]

use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use nudge_web::app::NudgeApp;

/// Write what winit would have reported for this frame.
fn set_focused(harness: &mut Harness<'_, NudgeApp>, focused: bool) {
    harness
        .input_mut()
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .expect("kittest always seeds the root viewport")
        .focused = Some(focused);
}

fn fields(harness: &Harness<'_, NudgeApp>) -> usize {
    harness.query_all_by_role(Role::TextInput).count()
}

/// One test, one harness: the three phases are one story, and the platform
/// globals the close writes to are shared across a test binary.
#[test]
fn a_card_arms_when_focus_arrives_and_closes_when_it_leaves() {
    // Ten minutes, so nothing here can be explained by the interval timer.
    let mut harness = Harness::builder()
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|cc| NudgeApp::new(cc, 10.0));

    // Phase 1 — the card is up but the window is not key yet. That is the
    // normal state right after a reveal: `activateIgnoringOtherApps` is a
    // request to the window server, and nothing on this thread has seen it
    // answered. A snapshot taken in `show_popup` reads exactly this.
    set_focused(&mut harness, false);
    harness.run_steps(5);
    assert_eq!(fields(&harness), 3, "the launch card is up");

    // …and it must stay up. An unarmed card is never closed by the absence of
    // a focus it has not been given yet — the bug this would introduce is a
    // card that closes itself the instant it opens.
    harness.run_steps(5);
    assert_eq!(
        fields(&harness),
        3,
        "a card that has not yet been given focus must not be closed for lacking it"
    );

    // Phase 2 — the activation lands, one or more frames late. This is what
    // arms the close; a snapshot-only implementation has already missed it.
    set_focused(&mut harness, true);
    harness.run_steps(3);
    assert_eq!(fields(&harness), 3, "still up, and now armed");

    // Phase 3 — Cmd-Tab away.
    set_focused(&mut harness, false);
    harness.run_steps(3);
    assert_eq!(
        fields(&harness),
        0,
        "focus loss closed the card (spec §4 Switch)"
    );
}
