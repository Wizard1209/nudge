#![cfg(target_os = "macos")]

use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use nudge_web::app::NudgeApp;

#[test]
fn the_card_comes_back_after_the_interval() {
    let mut harness = Harness::builder()
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|cc| NudgeApp::new(cc, 0.02));
    harness.run();
    assert_eq!(
        harness.query_all_by_role(Role::TextInput).count(),
        0,
        "launch is Waiting: no card"
    );

    std::thread::sleep(std::time::Duration::from_millis(1500));
    harness.run();
    assert_eq!(
        harness.get_all_by_role(Role::TextInput).count(),
        3,
        "the first interval elapsed and the card came up"
    );

    harness.key_press(egui::Key::Escape);
    harness.run();
    assert_eq!(
        harness.query_all_by_role(Role::TextInput).count(),
        0,
        "Esc dismissed the card"
    );

    std::thread::sleep(std::time::Duration::from_millis(1500));
    harness.run();
    assert_eq!(
        harness.get_all_by_role(Role::TextInput).count(),
        3,
        "the interval elapsed and the card came back"
    );
}
