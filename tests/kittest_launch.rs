//! Where the spec §4 machine starts, headless through the real `NudgeApp`:
//! Waiting, with the timer armed for the configured default interval, and
//! the popup arriving with the first Deadline.
//!
//! Its own test binary, like `kittest_viewport_commands.rs`: the armed timer
//! is the platform seam's, one per process, and every `NudgeApp` arms it at
//! launch. The tests here take [`SEAM`] for the same reason.

use std::sync::{Mutex, MutexGuard};
use std::time::{Duration, Instant};

use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use nudge_web::app::NudgeApp;
use nudge_web::platform;

/// Held for a whole test: the armed timer is per process.
static SEAM: Mutex<()> = Mutex::new(());

fn seam() -> MutexGuard<'static, ()> {
    SEAM.lock().unwrap_or_else(|e| e.into_inner())
}

fn launch(default_minutes: f64) -> Harness<'static, NudgeApp> {
    let mut harness = Harness::new_eframe(move |cc| NudgeApp::new(cc, default_minutes));
    harness.run();
    harness
}

fn fields(harness: &Harness<'_, NudgeApp>) -> usize {
    harness.query_all_by_role(Role::TextInput).count()
}

fn minutes(d: Duration) -> f64 {
    d.as_secs_f64() / 60.0
}

/// The armed interval and how much of it is left, read off the seam.
fn armed() -> (f64, f64) {
    let t = platform::armed_timer().expect("a timer is armed");
    let left = t.deadline.saturating_duration_since(Instant::now());
    (minutes(t.duration), minutes(left))
}

#[test]
fn launch_is_waiting_with_the_default_interval_armed() {
    let _seam = seam();
    let harness = launch(7.0);

    assert_eq!(fields(&harness), 0, "launch shows no popup");
    let (duration, left) = armed();
    assert_eq!(duration, 7.0, "armed for the configured default");
    assert!(left > 6.9, "the interval starts at launch: {left} min left");
}

#[test]
fn the_first_deadline_opens_a_timer_popup_on_the_first_field() {
    let _seam = seam();
    let mut harness = launch(7.0);

    harness.state_mut().deliver_deadline();
    harness.run();
    let inputs: Vec<_> = harness.get_all_by_role(Role::TextInput).collect();
    assert_eq!(inputs.len(), 3, "the Deadline opens the popup");
    assert!(inputs[0].is_focused(), "with the caret in the first field");
    assert_eq!(inputs[2].value().as_deref(), Some("7"));

    // Asking·Timer, not Asking·Manual: a Leave sets the timer from the
    // field, where a Leave from a manual popup would leave the launch
    // interval running.
    inputs[2].focus();
    harness.run();
    harness
        .get_all_by_role(Role::TextInput)
        .nth(2)
        .unwrap()
        .type_text("5");
    harness.run();
    harness.key_press(egui::Key::Escape);
    harness.run();

    assert_eq!(fields(&harness), 0, "Esc closed it");
    assert_eq!(armed().0, 75.0, "timer := field");
}
