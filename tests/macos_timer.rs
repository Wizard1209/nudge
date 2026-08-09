//! The interval timer, headless.
//!
//! `tests/macos_native.rs::popup_reappears_after_the_interval` is the
//! acceptance criterion for this behaviour, but it needs a logged-in GUI
//! session, an unlocked screen and an Accessibility grant — it cannot run in
//! CI, and it cannot run on a locked laptop. This test covers the half that
//! does not need any of that: the state machine, driven through the real
//! `NudgeApp` by `egui_kittest` exactly as `tests/kittest_card.rs` drives it.
//!
//! What it proves: `App::logic` -> `platform::tick` notices the deadline,
//! raises the fired flag, and `ui()` turns that into a `show_popup`. What it
//! cannot prove, and what the GUI test is still for: that eframe wakes a
//! *hidden* window on `request_repaint_after`, and that the AppKit
//! activation gives the revealed window the keyboard.
//!
//! Its own test binary on purpose. The platform seam holds its deadline in a
//! process-global (as `tray_bridge` does on Windows), so a sub-second timer
//! sharing a process with `kittest_card.rs`'s harnesses could fire inside one
//! of them. Separate integration test = separate process = no interference.

#![cfg(target_os = "macos")]

use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use nudge_web::app::NudgeApp;

/// The port's defining defect: the popup showed once at launch and never
/// came back, because the only native expiry path lived in the Windows tray
/// thread and `ui()` — the app's only implemented callback — is not called
/// while the window is hidden.
#[test]
fn the_card_comes_back_after_the_interval() {
    // 0.005 min = 300 ms. The constructor value is what lands in the
    // interval field, and the field is what the close reads back.
    let mut harness = Harness::new_eframe(|cc| NudgeApp::new(cc, 0.005));
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
        "Esc dismissed the card"
    );

    std::thread::sleep(std::time::Duration::from_millis(400));
    harness.run();
    assert_eq!(
        harness.get_all_by_role(Role::TextInput).count(),
        3,
        "the interval elapsed and the card came back"
    );
}
