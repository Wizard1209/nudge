#![cfg(target_os = "macos")]

use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use nudge_web::app::NudgeApp;

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

#[test]
fn a_card_arms_when_focus_arrives_and_closes_when_it_leaves() {
    let mut harness = Harness::builder()
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|cc| NudgeApp::new(cc, 10.0));
    nudge_web::platform::request_manual_open();

    set_focused(&mut harness, false);
    harness.run_steps(5);
    assert_eq!(fields(&harness), 3, "the card is up");

    harness.run_steps(5);
    assert_eq!(
        fields(&harness),
        3,
        "a card that has not yet been given focus must not be closed for lacking it"
    );

    set_focused(&mut harness, true);
    harness.run_steps(3);
    assert_eq!(fields(&harness), 3, "still up, and now armed");

    set_focused(&mut harness, false);
    harness.run_steps(3);
    assert_eq!(
        fields(&harness),
        0,
        "focus loss closed the card (spec §4 Switch)"
    );
}
