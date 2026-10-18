//! Headless AccessKit-driven tests against the real `NudgeApp` and
//! `SettingsApp`, via `egui_kittest::Harness::new_eframe`. No window is
//! opened and no display server is touched.
//!
//! `src/lib.rs` exposes `pub mod app;` unconditionally (it used to be
//! `#[cfg(target_arch = "wasm32")] mod app;`) purely so this integration
//! test crate — a separate compilation unit outside `nudge_web` — can reach
//! `NudgeApp`. `app.rs`'s own internal cfgs are untouched.
//!
//! ## Positional addressing, not labels
//!
//! The spotlight card's three fields are built with `TextEdit::hint_text`
//! only — no `ui.label()` + `labelled_by`, so the AccessKit tree carries no
//! accessible name for any of them (see `hint_text_alone_produces_no_accesskit_label`
//! in the probe this recipe came from). Adding labels would fix that, but it
//! would also change the card's layout, which the WASM e2e tier pixel-tests.
//! So every NudgeApp field below is addressed positionally, via
//! `get_all_by_role(Role::TextInput)` in draw order.

use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use nudge_web::app::NudgeApp;
use nudge_web::autostart::AutostartProvider;
use nudge_web::config::Config;
use nudge_web::settings_app::{PersistFn, SettingsApp, SettingsForm};

// =============================================================================
// NudgeApp — the spotlight card
// =============================================================================

fn nudge_harness<'a>(default_minutes: f64) -> Harness<'a, NudgeApp> {
    let mut harness = Harness::new_eframe(move |cc| NudgeApp::new(cc, default_minutes));
    harness.run();
    harness
}

/// Pins the phantom-fourth-Tab-stop regression: `egui::Area` implies a
/// focusable click widget unless `.interactable(false)` is set on it
/// (src/app.rs:340-343). Without that, the Area itself becomes a 4th
/// Tab stop between the real fields.
#[test]
fn boots_and_exposes_exactly_three_text_inputs() {
    let harness = nudge_harness(10.0);

    let inputs: Vec<_> = harness.get_all_by_role(Role::TextInput).collect();
    assert_eq!(inputs.len(), 3, "spotlight card has exactly three fields");

    // hint_text-only fields carry no AccessKit label.
    assert!(inputs.iter().all(|n| n.accesskit_node().label().is_none()));
}

#[test]
fn third_field_carries_the_constructor_interval() {
    let harness = nudge_harness(17.0);

    let inputs: Vec<_> = harness.get_all_by_role(Role::TextInput).collect();
    assert_eq!(inputs[2].value().as_deref(), Some("17"));
}

#[test]
fn focus_starts_on_the_first_field() {
    let harness = nudge_harness(10.0);

    let inputs: Vec<_> = harness.get_all_by_role(Role::TextInput).collect();
    assert!(
        inputs[0].is_focused(),
        "focus_first put the caret in field 1"
    );
    assert!(!inputs[1].is_focused());
    assert!(!inputs[2].is_focused());
}

#[test]
fn tab_walks_the_three_fields_in_order_and_does_not_escape_the_card() {
    let mut harness = nudge_harness(10.0);

    fn focused_index(harness: &Harness<'_, NudgeApp>) -> Option<usize> {
        harness
            .get_all_by_role(Role::TextInput)
            .position(|n| n.is_focused())
    }

    assert_eq!(focused_index(&harness), Some(0));

    harness.key_press(egui::Key::Tab);
    harness.run();
    assert_eq!(focused_index(&harness), Some(1), "Tab moved 1 -> 2");
    // Total field count is still exactly three: no phantom widget stole a
    // Tab stop between fields.
    assert_eq!(harness.get_all_by_role(Role::TextInput).count(), 3);

    harness.key_press(egui::Key::Tab);
    harness.run();
    assert_eq!(focused_index(&harness), Some(2), "Tab moved 2 -> 3");

    // One more Tab must land back on a real field of the card (index 0 or
    // 2, depending on whether egui wraps or the Area's own extra stop would
    // have absorbed it) — never on a fourth, nameless widget.
    harness.key_press(egui::Key::Tab);
    harness.run();
    assert_eq!(harness.get_all_by_role(Role::TextInput).count(), 3);
    assert!(
        focused_index(&harness).is_some(),
        "focus stayed on one of the card's three fields, did not escape it"
    );
}

#[test]
fn typing_round_trips_including_cyrillic() {
    let mut harness = nudge_harness(10.0);

    // `Node::type_text` does not focus the node — it queues an untargeted
    // text event that lands on whatever egui focus currently is. Field 1 is
    // already focused by `focus_first`, so no explicit `.focus()` is needed
    // here, but field 3 below needs one.
    harness
        .get_all_by_role(Role::TextInput)
        .next()
        .unwrap()
        .type_text("писать отчёт");
    harness.run();
    assert_eq!(
        harness
            .get_all_by_role(Role::TextInput)
            .next()
            .unwrap()
            .value()
            .as_deref(),
        Some("писать отчёт")
    );

    let third = harness.get_all_by_role(Role::TextInput).nth(2).unwrap();
    third.focus();
    harness.run();
    harness
        .get_all_by_role(Role::TextInput)
        .nth(2)
        .unwrap()
        .type_text("25");
    harness.run();
    assert_eq!(
        harness
            .get_all_by_role(Role::TextInput)
            .nth(2)
            .unwrap()
            .value()
            .as_deref(),
        Some("1025"), // field started at "10", typed text is inserted at caret 0 by kittest
    );
}

/// Escape runs the real Dismiss path. Enter (Submit) is deliberately never
/// exercised here: `hide_popup`'s submit branch calls
/// `journal::resolve_default_journal_path()` (src/journal.rs:93-96), which
/// has no injection point and writes straight into the developer's real
/// `~/Documents/Nudge/journal-rust.ndjson`.
///
/// TODO: covering Submit needs `journal::resolve_default_journal_path` (or
/// whatever writes the NDJSON line) to accept an injected path/writer the
/// way `SettingsApp` already accepts an injected `AutostartProvider` and
/// persist closure — that seam does not exist yet.
#[test]
fn escape_dismisses_the_card() {
    let mut harness = nudge_harness(10.0);
    assert_eq!(harness.get_all_by_role(Role::TextInput).count(), 3);

    harness.key_press(egui::Key::Escape);
    harness.run();

    assert_eq!(harness.query_all_by_role(Role::TextInput).count(), 0);
}

// =============================================================================
// SettingsApp
// =============================================================================

/// Autostart provider test double. Records every call so a test can assert
/// the toggle went through the injected seam rather than any real OS
/// mechanism. Held behind `Arc` so a clone can be kept for post-hoc
/// assertions after the original is moved into `SettingsApp::new`.
struct RecordingProvider {
    enabled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    enable_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    disable_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl RecordingProvider {
    fn new(initial: bool) -> Self {
        Self {
            enabled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(initial)),
            enable_calls: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
            disable_calls: std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        }
    }
}

impl AutostartProvider for RecordingProvider {
    fn enable(&self) -> Result<(), String> {
        self.enable_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.enabled
            .store(true, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    fn disable(&self) -> Result<(), String> {
        self.disable_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        self.enabled
            .store(false, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
    fn is_enabled(&self) -> Result<bool, String> {
        Ok(self.enabled.load(std::sync::atomic::Ordering::SeqCst))
    }
}

fn cfg(hotkey: &str, minutes: f64, autostart: bool) -> Config {
    Config {
        hotkey: hotkey.to_string(),
        default_interval_minutes: minutes,
        autostart,
        ..Config::default()
    }
}

fn no_op_persist() -> PersistFn {
    Box::new(|_cfg: &Config| Ok(()))
}

#[test]
fn settings_form_populates_from_a_config() {
    let provider: Box<dyn AutostartProvider> = Box::new(RecordingProvider::new(true));
    let mut harness = Harness::new_eframe(|cc| {
        SettingsApp::new(cc, cfg("Alt+J", 7.0, true), provider, no_op_persist())
    });
    harness.run();

    // Hotkey row, then interval row: both TextEdit::singleline with
    // hint_text only, same unlabelled shape as the spotlight card's fields
    // (row 1: "Global hotkey:", row 2: "Default interval (min):" are
    // separate ui.label()s, not wired via .labelled_by()).
    let inputs: Vec<_> = harness.get_all_by_role(Role::TextInput).collect();
    assert_eq!(inputs.len(), 2, "hotkey field + interval field");
    // The stored spelling is the cross-platform one; the field shows this
    // platform's — `⌥J` on macOS. Deriving the expectation the same way the
    // window does would assert nothing, so this states both forms.
    #[cfg(target_os = "macos")]
    assert_eq!(inputs[0].value().as_deref(), Some("⌥J"));
    #[cfg(not(target_os = "macos"))]
    assert_eq!(inputs[0].value().as_deref(), Some("Alt+J"));
    assert_eq!(inputs[1].value().as_deref(), Some("7"));

    let checkbox = harness.get_by_role(Role::CheckBox);
    assert_eq!(
        checkbox.accesskit_node().toggled(),
        Some(eframe::egui::accesskit::Toggled::True),
        "autostart=true from the config reflects into the checkbox"
    );
}

/// `SettingsForm` (src/settings_app.rs) is the pure, no-eframe, no-I/O state
/// machine `SettingsApp` wraps — its own module doc calls it out as "what's
/// unit-tested". `is_dirty` is exercised here directly rather than through
/// the eframe shell because `SettingsApp`'s `form` field is private to its
/// module: an integration test, being a separate crate, cannot reach it
/// through `Harness::state()`, and adding an accessor would touch `src/`
/// beyond what this task allows. The typing-round-trips-through-the-UI half
/// of "editing" is already covered for the shared TextEdit machinery by
/// `typing_round_trips_including_cyrillic` above and by
/// `settings_app_editing_a_field_round_trips_through_the_ui` below.
#[test]
fn settings_form_editing_marks_it_dirty() {
    let mut form = SettingsForm::from_config(cfg("Ctrl+Shift+Space", 10.0, false));
    assert!(!form.is_dirty(), "freshly-loaded form is clean");

    form.interval_text = "25".to_string();
    assert!(form.is_dirty(), "editing a field marks the form dirty");
}

#[test]
fn settings_app_editing_a_field_round_trips_through_the_ui() {
    let provider: Box<dyn AutostartProvider> = Box::new(RecordingProvider::new(false));
    let mut harness = Harness::new_eframe(|cc| {
        SettingsApp::new(cc, cfg("Alt+J", 7.0, false), provider, no_op_persist())
    });
    harness.run();

    let interval_field = harness.get_all_by_role(Role::TextInput).nth(1).unwrap();
    interval_field.focus();
    harness.run();
    harness
        .get_all_by_role(Role::TextInput)
        .nth(1)
        .unwrap()
        .type_text("5");
    harness.run();

    assert_eq!(
        harness
            .get_all_by_role(Role::TextInput)
            .nth(1)
            .unwrap()
            .value()
            .as_deref(),
        Some("75"), // "7" with "5" inserted at the caret (index 0)
    );
}

#[test]
fn toggling_autostart_goes_through_the_injected_provider_not_the_os() {
    let provider = RecordingProvider::new(false);
    let enable_calls = std::sync::Arc::clone(&provider.enable_calls);
    let enabled = std::sync::Arc::clone(&provider.enabled);

    let persisted: std::sync::Arc<std::sync::Mutex<Vec<Config>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let persisted_writer = std::sync::Arc::clone(&persisted);
    let persist: PersistFn = Box::new(move |cfg: &Config| {
        persisted_writer.lock().unwrap().push(cfg.clone());
        Ok(())
    });

    let boxed_provider: Box<dyn AutostartProvider> = Box::new(provider);
    let mut harness = Harness::new_eframe(|cc| {
        SettingsApp::new(cc, cfg("Alt+J", 7.0, false), boxed_provider, persist)
    });
    harness.run();

    assert_eq!(
        enable_calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "unclicked checkbox never touches the provider"
    );

    harness.get_by_role(Role::CheckBox).click();
    harness.run();

    assert_eq!(
        enable_calls.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "toggling the checkbox calls the injected provider's enable(), not any OS API"
    );
    assert!(enabled.load(std::sync::atomic::Ordering::SeqCst));
    assert_eq!(
        persisted.lock().unwrap().last().map(|c| c.autostart),
        Some(true),
        "the confirmed autostart change is persisted through the injected closure"
    );
}
