//! The manual way back to the card, headless.
//!
//! Three things summon the popup by hand on macOS — a left click on the
//! menu-bar status item, the "Show Nudge" menu entry, and the global hotkey —
//! and all three converge on one edge: `platform::set_tray_clicked`, raised
//! from `platform::tick`, consumed by `App::ui`. This test drives that edge
//! directly, so the half of the path that lives in the app is covered without a
//! window server, a status item, or a keystroke.
//!
//! What it cannot cover, and what `tests/macos_native.rs` and a real unlocked
//! screen are still for: that AppKit delivers the click, that Carbon delivers
//! the hotkey, and that the revealed window actually takes the keyboard.
//! `platform::macos`'s own unit tests cover the middle link — `tick` turning a
//! handler's flag into this edge.
//!
//! Its own test binary for the same reason `macos_timer.rs` is: the platform
//! seam holds `TRAY_CLICKED` and the popup-visible flag in process globals, so
//! a second harness in the same process could consume this one's edge.

#![cfg(target_os = "macos")]

use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use nudge_web::app::NudgeApp;

/// Before this landed, `TriggerSource::Manual` was dead code on macOS: nothing
/// on the platform could raise the edge that produces a manual open, so the
/// only way the card ever came back was the timer.
#[test]
fn a_manual_trigger_brings_a_dismissed_card_back() {
    // Ten minutes, so the interval timer cannot be what reopens the card and
    // pass this test for the wrong reason.
    let mut harness = Harness::builder()
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|cc| NudgeApp::new(cc, 10.0));
    harness.run();
    assert_eq!(
        harness.get_all_by_role(Role::TextInput).count(),
        3,
        "launch popup is up"
    );

    harness.key_press(egui::Key::Escape);
    harness.run();
    assert_eq!(
        harness.query_all_by_role(Role::TextInput).count(),
        0,
        "Esc dismissed the card, and the ten-minute timer will not bring it back"
    );

    nudge_web::platform::set_tray_clicked();
    harness.run();
    assert_eq!(
        harness.get_all_by_role(Role::TextInput).count(),
        3,
        "a status-item click / hotkey press must reopen the hidden card"
    );
}
