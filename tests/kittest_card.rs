use eframe::egui;
use eframe::egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

use nudge_web::app::NudgeApp;
use nudge_web::autostart::AutostartProvider;
use nudge_web::config::Config;
use nudge_web::journal::JournalEvent;
use nudge_web::recorder::{self, EdgeSource, PhysicalKey, PressEdge, Verdict};
use nudge_web::settings_app::{
    MARGIN, PersistFn, SettingsApp, SettingsForm, SettingsHost, SpriteCheckFn, UpdateHost,
    WINDOW_SIZE,
};
use nudge_web::update::{self, Action, Release};

fn nudge_harness<'a>(default_minutes: f64) -> Harness<'a, NudgeApp> {
    let mut harness = Harness::new_eframe(move |cc| NudgeApp::new(cc, default_minutes));
    harness.run();
    first_deadline(&mut harness);
    harness
}

fn first_deadline(harness: &mut Harness<'_, NudgeApp>) {
    harness.state_mut().deliver_deadline();
    harness.run();
}

#[test]
fn boots_and_exposes_exactly_three_text_inputs() {
    let harness = nudge_harness(10.0);

    let inputs: Vec<_> = harness.get_all_by_role(Role::TextInput).collect();
    assert_eq!(inputs.len(), 3, "spotlight card has exactly three fields");

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
    assert_eq!(harness.get_all_by_role(Role::TextInput).count(), 3);

    harness.key_press(egui::Key::Tab);
    harness.run();
    assert_eq!(focused_index(&harness), Some(2), "Tab moved 2 -> 3");

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
        Some("1025"),
    );
}

#[test]
fn ctrl_backspace_deletes_one_cyrillic_word_once() {
    let mut harness = nudge_harness(10.0);
    type_into_card(&mut harness, 0, "привет мир");

    harness.key_press_modifiers(
        egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
        egui::Key::Backspace,
    );
    harness.run();

    assert_eq!(
        harness
            .get_all_by_role(Role::TextInput)
            .next()
            .unwrap()
            .value()
            .as_deref(),
        Some("привет "),
    );
}

#[test]
fn enter_submits_one_journal_event() {
    let events: std::rc::Rc<std::cell::RefCell<Vec<JournalEvent>>> = Default::default();
    let recorder = std::rc::Rc::clone(&events);
    let mut harness = Harness::new_eframe(move |cc| {
        NudgeApp::new(cc, 10.0).with_journal_sink(Box::new(move |event| {
            recorder.borrow_mut().push(event.clone());
            Ok(())
        }))
    });
    harness.run();
    first_deadline(&mut harness);

    harness
        .get_all_by_role(Role::TextInput)
        .next()
        .unwrap()
        .type_text("писать отчёт");
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();

    let events = events.borrow();
    assert_eq!(events.len(), 1, "one Submit, one line");
    assert_eq!(events[0].doing, "писать отчёт");
    assert_eq!(events[0].trigger_source, "timer");
    assert_eq!(events[0].next_interval_minutes, 10.0);
}

#[test]
fn escape_dismisses_the_card() {
    let mut harness = nudge_harness(10.0);
    assert_eq!(harness.get_all_by_role(Role::TextInput).count(), 3);

    harness.key_press(egui::Key::Escape);
    harness.run();

    assert_eq!(harness.query_all_by_role(Role::TextInput).count(), 0);
}

fn recording_nudge_harness<'a>(
    default_minutes: f64,
) -> (
    Harness<'a, NudgeApp>,
    std::rc::Rc<std::cell::RefCell<Vec<JournalEvent>>>,
) {
    let events: std::rc::Rc<std::cell::RefCell<Vec<JournalEvent>>> = Default::default();
    let recorder = std::rc::Rc::clone(&events);
    let mut harness = Harness::new_eframe(move |cc| {
        NudgeApp::new(cc, default_minutes).with_journal_sink(Box::new(move |event| {
            recorder.borrow_mut().push(event.clone());
            Ok(())
        }))
    });
    harness.run();
    first_deadline(&mut harness);
    (harness, events)
}

fn type_into_card(harness: &mut Harness<'_, NudgeApp>, nth: usize, text: &str) {
    harness
        .get_all_by_role(Role::TextInput)
        .nth(nth)
        .unwrap()
        .focus();
    harness.run();
    harness
        .get_all_by_role(Role::TextInput)
        .nth(nth)
        .unwrap()
        .type_text(text);
    harness.run();
}

#[test]
fn submitting_an_interval_too_large_to_arm_is_refused() {
    let (mut harness, events) = recording_nudge_harness(1.0);
    type_into_card(&mut harness, 0, "doing");
    type_into_card(&mut harness, 2, "e20");
    let inputs: Vec<_> = harness.get_all_by_role(Role::TextInput).collect();
    assert_eq!(inputs[2].value().as_deref(), Some("1e20"));

    harness.key_press(egui::Key::Enter);
    harness.run();

    assert_eq!(
        harness.get_all_by_role(Role::TextInput).count(),
        3,
        "the popup stays"
    );
    assert!(
        harness
            .query_by_label(
                "Interval must be more than 0 and at most 1000000 minutes (got: \"1e20\")"
            )
            .is_some(),
        "the popup says why"
    );
    assert!(
        events.borrow().is_empty(),
        "a refused submit wrote something"
    );
}

#[test]
fn leaving_with_an_interval_too_large_to_arm_closes() {
    let (mut harness, events) = recording_nudge_harness(1.0);
    type_into_card(&mut harness, 2, "e20");
    assert_eq!(
        harness
            .get_all_by_role(Role::TextInput)
            .nth(2)
            .unwrap()
            .value()
            .as_deref(),
        Some("1e20")
    );

    let pos = egui::pos2(2.0, 2.0);
    harness.event(egui::Event::PointerMoved(pos));
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    harness.run();

    assert_eq!(
        harness.query_all_by_role(Role::TextInput).count(),
        0,
        "the popup closed"
    );
    assert!(events.borrow().is_empty());
}

#[test]
fn a_configured_interval_too_large_to_arm_starts_the_popup_on_the_default() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("config.json");
    std::fs::write(&path, r#"{"default_interval_minutes":1e20}"#).unwrap();
    let (config, err) = nudge_web::config::load_or_default(&path);
    assert!(err.is_none(), "{err:?}");
    assert_eq!(config.default_interval_minutes, 1e20);

    let (minutes, was_invalid) = config.resolved_interval_minutes();
    assert!(was_invalid);
    let (mut harness, events) = recording_nudge_harness(minutes);
    assert_eq!(
        harness
            .get_all_by_role(Role::TextInput)
            .nth(2)
            .unwrap()
            .value()
            .as_deref(),
        Some("10")
    );

    type_into_card(&mut harness, 0, "doing");
    harness.key_press(egui::Key::Enter);
    harness.run();

    let events = events.borrow();
    assert_eq!(events.len(), 1);
    assert_eq!(events[0].next_interval_minutes, 10.0);
}

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

#[cfg(target_os = "macos")]
const AUTOSTART_LABEL: &str = "Open at login";
#[cfg(not(target_os = "macos"))]
const AUTOSTART_LABEL: &str = "Launch with Windows";

#[test]
fn settings_form_populates_from_a_config() {
    let provider: Box<dyn AutostartProvider> = Box::new(RecordingProvider::new(true));
    let mut harness = Harness::new_eframe(|cc| {
        SettingsApp::new(
            cc,
            cfg("Alt+J", 7.0, true),
            SettingsHost::headless(provider),
        )
    });
    harness.run();

    let inputs: Vec<_> = harness.get_all_by_role(Role::TextInput).collect();
    assert_eq!(
        inputs.len(),
        6,
        "hotkey, interval, sprite path, X, Y, card width"
    );
    #[cfg(target_os = "macos")]
    assert_eq!(inputs[0].value().as_deref(), Some("⌥J"));
    #[cfg(not(target_os = "macos"))]
    assert_eq!(inputs[0].value().as_deref(), Some("Alt+J"));
    assert_eq!(inputs[1].value().as_deref(), Some("7"));

    let checkbox = harness.get_by_label(AUTOSTART_LABEL);
    assert_eq!(
        checkbox.accesskit_node().toggled(),
        Some(eframe::egui::accesskit::Toggled::True),
        "autostart=true from the config reflects into the checkbox"
    );
}

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
        SettingsApp::new(
            cc,
            cfg("Alt+J", 7.0, false),
            SettingsHost::headless(provider),
        )
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
        Some("75"),
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
        SettingsApp::new(
            cc,
            cfg("Alt+J", 7.0, false),
            SettingsHost {
                persist,
                ..SettingsHost::headless(boxed_provider)
            },
        )
    });
    harness.run();

    assert_eq!(
        enable_calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "unclicked checkbox never touches the provider"
    );

    harness.get_by_label(AUTOSTART_LABEL).click();
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

#[test]
fn settings_save_with_a_bad_sprite_shows_the_reason_and_persists_nothing() {
    const REASON: &str = "Sprite PNG has no transparent pixels — it needs a transparent background";

    let persisted: std::sync::Arc<std::sync::Mutex<Vec<Config>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let persisted_writer = std::sync::Arc::clone(&persisted);
    let persist: PersistFn = Box::new(move |cfg: &Config| {
        persisted_writer.lock().unwrap().push(cfg.clone());
        Ok(())
    });
    let refuse: SpriteCheckFn = Box::new(|_path: &std::path::Path| Err(REASON.to_string()));

    let provider: Box<dyn AutostartProvider> = Box::new(RecordingProvider::new(false));
    let mut harness = Harness::new_eframe(|cc| {
        SettingsApp::new(
            cc,
            cfg("Alt+J", 7.0, false),
            SettingsHost {
                persist,
                sprite_check: refuse,
                ..SettingsHost::headless(provider)
            },
        )
    });
    harness.run();

    harness
        .get_all_by_role(Role::TextInput)
        .nth(2)
        .unwrap()
        .focus();
    harness.run();
    harness
        .get_all_by_role(Role::TextInput)
        .nth(2)
        .unwrap()
        .type_text("/art/s.png");
    harness.run();
    harness.get_by_label("Sprite").click();
    harness.run();
    harness.get_by_label("Save").click();
    harness.run();

    assert!(
        harness.query_by_label(REASON).is_some(),
        "the message line carries the check's reason"
    );
    assert!(
        persisted.lock().unwrap().is_empty(),
        "a refused save writes nothing"
    );
    assert!(
        harness.query_by_label("Saved").is_none(),
        "a refused save must not say Saved"
    );
}

fn settings_harness<'a>(
    config: Config,
) -> (
    Harness<'a, SettingsApp>,
    std::sync::Arc<std::sync::Mutex<Vec<Config>>>,
) {
    let persisted: std::sync::Arc<std::sync::Mutex<Vec<Config>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let writer = std::sync::Arc::clone(&persisted);
    let persist: PersistFn = Box::new(move |cfg: &Config| {
        writer.lock().unwrap().push(cfg.clone());
        Ok(())
    });
    let provider: Box<dyn AutostartProvider> = Box::new(RecordingProvider::new(false));
    let mut harness = Harness::builder()
        .with_size(WINDOW_SIZE)
        .build_eframe(move |cc| {
            SettingsApp::new(
                cc,
                config,
                SettingsHost {
                    persist,
                    ..SettingsHost::headless(provider)
                },
            )
        });
    harness.run();
    (harness, persisted)
}

#[derive(Clone, Default)]
struct FakeEdgeSource {
    edges: std::sync::Arc<std::sync::Mutex<Vec<PressEdge>>>,
    armed: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl EdgeSource for FakeEdgeSource {
    fn drain(&mut self) -> Vec<PressEdge> {
        std::mem::take(&mut *self.edges.lock().unwrap())
    }
    fn set_armed(&mut self, armed: bool) {
        self.armed.store(armed, std::sync::atomic::Ordering::SeqCst);
    }
    fn classify(&self, _edge: PressEdge) -> Verdict {
        Verdict::NotBindable {
            name: "Enter".to_string(),
        }
    }
}

fn settings_harness_with_source<'a>(
    config: Config,
    source: FakeEdgeSource,
) -> (
    Harness<'a, SettingsApp>,
    std::sync::Arc<std::sync::Mutex<Vec<Config>>>,
) {
    let persisted: std::sync::Arc<std::sync::Mutex<Vec<Config>>> =
        std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let writer = std::sync::Arc::clone(&persisted);
    let persist: PersistFn = Box::new(move |cfg: &Config| {
        writer.lock().unwrap().push(cfg.clone());
        Ok(())
    });
    let provider: Box<dyn AutostartProvider> = Box::new(RecordingProvider::new(false));
    let mut harness = Harness::builder()
        .with_size(WINDOW_SIZE)
        .build_eframe(move |_cc| {
            SettingsApp::with_source(
                config,
                SettingsHost {
                    persist,
                    ..SettingsHost::headless(provider)
                },
                Some(Box::new(source)),
            )
        });
    harness.run();
    (harness, persisted)
}

fn type_into(harness: &mut Harness<'_, SettingsApp>, nth: usize, text: &str) {
    harness
        .get_all_by_role(Role::TextInput)
        .nth(nth)
        .unwrap()
        .focus();
    harness.run();
    harness
        .get_all_by_role(Role::TextInput)
        .nth(nth)
        .unwrap()
        .type_text(text);
    harness.run();
}

fn save_is_disabled(harness: &Harness<'_, SettingsApp>) -> bool {
    harness.get_by_label("Save").accesskit_node().is_disabled()
}

fn says_saved(harness: &Harness<'_, SettingsApp>) -> bool {
    harness.query_by_label("Saved").is_some()
}

#[test]
fn settings_return_in_a_field_saves() {
    let (mut harness, persisted) = settings_harness(cfg("Alt+J", 7.0, false));

    type_into(&mut harness, 1, "5");
    harness.key_press(egui::Key::Enter);
    harness.run();
    assert_eq!(
        persisted
            .lock()
            .unwrap()
            .last()
            .map(|c| c.default_interval_minutes),
        Some(75.0),
        "Return in the interval field did not write the form"
    );
    assert!(says_saved(&harness), "a save from Return says Saved too");

    type_into(&mut harness, 3, "4");
    harness.key_press(egui::Key::Enter);
    harness.run();
    assert_eq!(
        persisted.lock().unwrap().last().map(|c| c.sprite.x),
        Some(4),
        "Return in the sprite X field did not write the form"
    );
}

#[test]
fn settings_return_runs_the_same_checks_and_shows_the_same_message() {
    let (mut harness, persisted) = settings_harness(cfg("Alt+J", 7.0, false));

    type_into(&mut harness, 1, "x");
    harness.key_press(egui::Key::Enter);
    harness.run();

    assert!(
        harness
            .query_by_label(
                "Interval must be more than 0 and at most 1000000 minutes (got: \"7x\")"
            )
            .is_some(),
        "the message line does not carry the interval check's reason"
    );
    assert!(
        persisted.lock().unwrap().is_empty(),
        "a refused save from Return wrote something"
    );
    assert!(!says_saved(&harness), "a refused save must not say Saved");
}

#[test]
fn settings_refuses_an_interval_too_large_to_arm() {
    let (mut harness, persisted) = settings_harness(cfg("Alt+J", 1.0, false));

    type_into(&mut harness, 1, "e20");
    harness.key_press(egui::Key::Enter);
    harness.run();

    assert!(
        harness
            .query_by_label(
                "Interval must be more than 0 and at most 1000000 minutes (got: \"1e20\")"
            )
            .is_some(),
        "the message line does not carry the interval check's reason"
    );
    assert!(
        persisted.lock().unwrap().is_empty(),
        "an interval too large to arm was written"
    );
}

#[test]
fn settings_return_on_a_clean_form_does_nothing() {
    let (mut harness, persisted) = settings_harness(cfg("Alt+J", 7.0, false));

    harness
        .get_all_by_role(Role::TextInput)
        .nth(1)
        .unwrap()
        .focus();
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();

    assert!(
        persisted.lock().unwrap().is_empty(),
        "Return on an unedited form wrote the file"
    );
    assert!(!says_saved(&harness));
}

#[test]
fn settings_return_while_the_recorder_is_armed_is_a_press_not_a_save() {
    let source = FakeEdgeSource::default();
    let (mut harness, persisted) =
        settings_harness_with_source(cfg("Alt+J", 7.0, false), source.clone());

    type_into(&mut harness, 1, "5");
    harness.get_by_label("Record").click();
    harness.run();
    assert!(
        harness.query_all_by_label(recorder::PROMPT).count() > 0,
        "Record did not arm the recorder"
    );
    assert!(source.armed.load(std::sync::atomic::Ordering::SeqCst));

    source.edges.lock().unwrap().push(PressEdge {
        key: PhysicalKey(36),
        modifiers: nudge_web::hotkey::Modifiers::NONE,
    });
    harness.key_press(egui::Key::Enter);
    harness.run();

    assert!(
        persisted.lock().unwrap().is_empty(),
        "Return while armed wrote the file"
    );
    assert!(!says_saved(&harness));
    assert!(
        harness
            .query_all_by_label(
                "Enter cannot be part of a hotkey. Press modifiers plus one key. Esc cancels."
            )
            .count()
            > 0,
        "the press was not the recorder's"
    );
    assert!(
        source.armed.load(std::sync::atomic::Ordering::SeqCst),
        "a refused key must leave the recorder armed"
    );
}

#[test]
fn settings_return_on_a_focused_checkbox_is_the_checkbox_s() {
    let (mut harness, persisted) = settings_harness(cfg("Alt+J", 7.0, false));

    type_into(&mut harness, 1, "5");
    harness.get_by_label(AUTOSTART_LABEL).focus();
    harness.run();
    harness.key_press(egui::Key::Enter);
    harness.run();

    assert!(
        persisted
            .lock()
            .unwrap()
            .iter()
            .all(|c| c.default_interval_minutes == 7.0),
        "Return on the checkbox saved the form"
    );
    assert!(!says_saved(&harness));
    assert!(!save_is_disabled(&harness), "the edit is still unsaved");
}

#[test]
fn settings_says_saved_after_a_save_until_the_first_edit() {
    let (mut harness, _persisted) = settings_harness(cfg("Alt+J", 7.0, false));
    assert!(!says_saved(&harness), "nothing has been saved yet");

    type_into(&mut harness, 1, "5");
    harness.get_by_label("Save").click();
    harness.run();
    assert!(says_saved(&harness), "a save that reached the file says so");

    type_into(&mut harness, 1, "0");
    assert!(
        !says_saved(&harness),
        "the first edit after a save must take the note away"
    );
}

#[test]
fn settings_autostart_click_after_a_save_takes_the_note_away_and_raises_none() {
    let (mut harness, persisted) = settings_harness(cfg("Alt+J", 7.0, false));

    type_into(&mut harness, 1, "5");
    harness.get_by_label("Save").click();
    harness.run();
    assert!(says_saved(&harness));

    harness.get_by_label(AUTOSTART_LABEL).click();
    harness.run();
    assert!(
        !says_saved(&harness),
        "an autostart click is an edit: the note must go"
    );
    assert_eq!(
        persisted.lock().unwrap().last().map(|c| c.autostart),
        Some(true),
        "the click itself was committed"
    );
    assert!(
        save_is_disabled(&harness),
        "nothing differs from the file after the click"
    );
}

#[test]
fn settings_save_is_greyed_while_the_form_matches_the_file() {
    let (mut harness, persisted) = settings_harness(cfg("Alt+J", 7.0, false));
    assert!(
        save_is_disabled(&harness),
        "a freshly-loaded form has nothing to save"
    );

    type_into(&mut harness, 1, "5");
    assert!(!save_is_disabled(&harness), "an edit makes Save live");

    harness.key_press(egui::Key::Backspace);
    harness.run();
    assert_eq!(
        harness
            .get_all_by_role(Role::TextInput)
            .nth(1)
            .unwrap()
            .value()
            .as_deref(),
        Some("7")
    );
    assert!(
        save_is_disabled(&harness),
        "typing the stored value back is no edit"
    );

    type_into(&mut harness, 1, "5");
    harness.get_by_label("Save").click();
    harness.run();
    assert_eq!(persisted.lock().unwrap().len(), 1);
    assert!(
        save_is_disabled(&harness),
        "a just-saved form reads as unedited, and its content is on disk"
    );

    harness.get_by_label("Save").click();
    harness.run();
    assert_eq!(persisted.lock().unwrap().len(), 1);
}

use egui_kittest::kittest::By;

#[cfg(target_os = "macos")]
const AUTOSTART_ROW: &str = "Open at login";
#[cfg(not(target_os = "macos"))]
const AUTOSTART_ROW: &str = "Launch with Windows";

fn rect_of(harness: &Harness<'_, SettingsApp>, label: &str) -> egui::Rect {
    harness
        .query_by_label(label)
        .unwrap_or_else(|| panic!("no widget labelled {label:?} in the settings window"))
        .rect()
}

fn row_text_rect(harness: &Harness<'_, SettingsApp>, text: &str) -> egui::Rect {
    harness
        .query_all(By::new().role(Role::Label).label(text).include_labels())
        .next()
        .unwrap_or_else(|| panic!("no row text {text:?} in the settings window"))
        .rect()
}

fn input_rect(harness: &Harness<'_, SettingsApp>, nth: usize) -> egui::Rect {
    harness
        .get_all_by_role(Role::TextInput)
        .nth(nth)
        .unwrap_or_else(|| panic!("fewer than {} text inputs", nth + 1))
        .rect()
}

fn checkbox_rects(harness: &Harness<'_, SettingsApp>) -> Vec<egui::Rect> {
    let rects: Vec<egui::Rect> = harness
        .get_all_by_role(Role::CheckBox)
        .map(|n| n.rect())
        .collect();
    assert_eq!(rects.len(), 2, "two checkboxes: autostart and sprite");
    rects
}

fn label_texts(harness: &Harness<'_, SettingsApp>) -> Vec<String> {
    harness
        .get_all_by_role(Role::Label)
        .filter_map(|n| n.value())
        .collect()
}

fn one_edge(edges: &[(&str, f32)]) -> bool {
    let min = edges.iter().map(|e| e.1).fold(f32::MAX, f32::min);
    let max = edges.iter().map(|e| e.1).fold(f32::MIN, f32::max);
    max - min <= 1.0
}

fn focused_widget(harness: &Harness<'_, SettingsApp>) -> (String, egui::Rect) {
    let node = harness
        .query_all(By::new().predicate(|n| n.is_focused()))
        .next()
        .expect("a widget has keyboard focus");
    let name = format!(
        "{:?} {:?}",
        node.accesskit_node().role(),
        node.accesskit_node().label().or(node.value())
    );
    (name, node.rect())
}

#[test]
fn settings_tab_visits_controls_in_reading_order() {
    let (mut harness, _) =
        settings_harness_with_source(cfg("Alt+J", 10.0, false), FakeEdgeSource::default());
    harness
        .get_all_by_role(Role::TextInput)
        .next()
        .unwrap()
        .focus();
    harness.run();

    let mut visited = vec![focused_widget(&harness)];
    for _ in 0..30 {
        harness.key_press(egui::Key::Tab);
        harness.run();
        let next = focused_widget(&harness);
        if next.1 == visited[0].1 {
            break;
        }
        visited.push(next);
    }
    assert!(
        visited.len() >= 8,
        "Tab reached too few controls: {visited:?}"
    );

    for pair in visited.windows(2) {
        let (a, b) = (&pair[0], &pair[1]);
        let same_row = (a.1.center().y - b.1.center().y).abs() < 1.0;
        let in_order = if same_row {
            b.1.left() > a.1.left()
        } else {
            b.1.center().y > a.1.center().y
        };
        assert!(in_order, "Tab went from {} to {}", a.0, b.0);
    }
}

#[test]
fn settings_has_no_heading_repeating_the_title() {
    let (harness, _persisted) = settings_harness(cfg("Alt+J", 7.0, false));
    assert!(
        harness.query_by_label("Settings").is_none(),
        "a widget reads \"Settings\" inside the window — the title bar already carries it"
    );
    assert!(
        harness
            .query_by_label_contains("Nudge — Settings")
            .is_none(),
        "the window's title is repeated inside it"
    );
}

#[test]
fn settings_cancel_then_save_at_the_bottom_right() {
    let (harness, _persisted) = settings_harness(cfg("Alt+J", 7.0, false));
    let cancel = rect_of(&harness, "Cancel");
    let save = rect_of(&harness, "Save");
    assert!(
        cancel.max.x <= save.min.x + 0.5,
        "Cancel spans x={}..{} and Save x={}..{}: Cancel is not left of Save",
        cancel.min.x,
        cancel.max.x,
        save.min.x,
        save.max.x
    );
    assert!(
        (cancel.center().y - save.center().y).abs() <= 1.0,
        "Cancel and Save are not on one line"
    );

    let mut right = f32::MIN;
    let mut bottom = f32::MIN;
    for role in [Role::TextInput, Role::Button, Role::CheckBox, Role::Label] {
        for node in harness.get_all_by_role(role) {
            right = right.max(node.rect().max.x);
            bottom = bottom.max(node.rect().max.y);
        }
    }
    assert!(
        save.max.x >= right - 1.0,
        "Save ends at x={} but the form reaches x={right}: Save is not at the right",
        save.max.x
    );
    assert!(
        save.max.y >= bottom - 1.0,
        "Save ends at y={} but the form reaches y={bottom}: Save is not at the bottom",
        save.max.y
    );
    let box_right = WINDOW_SIZE[0] - f32::from(MARGIN);
    assert!(
        (save.max.x - box_right).abs() <= 1.0,
        "Save ends at x={} and the boxes at x={box_right}: the buttons do not end where the boxes do",
        save.max.x
    );
}

#[test]
fn settings_saved_note_sits_left_of_cancel() {
    let (mut harness, _persisted) = settings_harness(cfg("Alt+J", 7.0, false));
    type_into(&mut harness, 1, "5");
    harness.get_by_label("Save").click();
    harness.run();
    assert!(says_saved(&harness), "the save did not say Saved");

    let saved = rect_of(&harness, "Saved");
    let cancel = rect_of(&harness, "Cancel");
    assert!(
        saved.max.x <= cancel.min.x + 0.5,
        "Saved spans x={}..{} and Cancel x={}..{}: the note is not to the buttons' left",
        saved.min.x,
        saved.max.x,
        cancel.min.x,
        cancel.max.x
    );
    assert!(
        saved.min.y < cancel.max.y && cancel.min.y < saved.max.y,
        "Saved is not on the buttons' line"
    );
}

#[test]
fn settings_row_texts_share_a_left_edge_and_controls_a_right_edge() {
    let (harness, _persisted) = settings_harness(cfg("Alt+J", 7.0, false));

    let texts = [
        "Global hotkey",
        "Default interval",
        AUTOSTART_ROW,
        "Sprite",
        "PNG",
        "Position (pt)",
    ];
    let lefts: Vec<(&str, f32)> = texts
        .iter()
        .map(|t| (*t, row_text_rect(&harness, t).min.x))
        .collect();

    let checkboxes = checkbox_rects(&harness);
    let hotkey_end = harness
        .query_by_label("Record")
        .map(|n| n.rect())
        .unwrap_or_else(|| input_rect(&harness, 0));
    let png_end = harness
        .query_by_label("Browse…")
        .map(|n| n.rect())
        .unwrap_or_else(|| input_rect(&harness, 2));
    let rights = [
        ("hotkey", hotkey_end.max.x),
        ("interval (min)", rect_of(&harness, "min").max.x),
        ("autostart", checkboxes[0].max.x),
        ("sprite", checkboxes[1].max.x),
        ("PNG", png_end.max.x),
        ("position", input_rect(&harness, 5).max.x),
    ];

    assert!(
        one_edge(&lefts),
        "the row texts start at {lefts:?}, not at one left edge"
    );
    assert!(
        one_edge(&rights),
        "the rows' last controls end at {rights:?}, not at one right edge"
    );
}

#[test]
fn settings_checkboxes_sit_at_the_right_of_their_rows() {
    let (harness, _persisted) = settings_harness(cfg("Alt+J", 7.0, false));
    let checkboxes = checkbox_rects(&harness);
    let right_edge = harness
        .query_by_label("Record")
        .map(|n| n.rect())
        .unwrap_or_else(|| input_rect(&harness, 0))
        .max
        .x;
    for (text, checkbox) in [AUTOSTART_ROW, "Sprite"].into_iter().zip(checkboxes) {
        let label = row_text_rect(&harness, text);
        assert!(
            label.max.x <= checkbox.min.x,
            "the {text:?} checkbox at x={} is not to the right of its text, which ends at x={}",
            checkbox.min.x,
            label.max.x
        );
        assert!(
            (checkbox.max.x - right_edge).abs() <= 1.0,
            "the {text:?} checkbox ends at x={}, the form's right edge is at x={right_edge}",
            checkbox.max.x
        );
    }
}

#[test]
fn settings_vocabulary_is_a_footnote_under_the_hotkey_group() {
    let (harness, _persisted) = settings_harness(cfg("Alt+J", 7.0, false));
    let vocabulary = rect_of(&harness, nudge_web::hotkey::VOCABULARY);
    let hotkey = input_rect(&harness, 0);
    let interval = input_rect(&harness, 1);
    assert!(
        vocabulary.min.y >= hotkey.max.y - 0.5,
        "the vocabulary (y={}) is not below the hotkey row (ending y={})",
        vocabulary.min.y,
        hotkey.max.y
    );
    assert!(
        vocabulary.max.y <= interval.min.y + 0.5,
        "the vocabulary (ending y={}) is not above the interval row (y={})",
        vocabulary.max.y,
        interval.min.y
    );
    assert!(
        vocabulary.min.y - hotkey.max.y < interval.min.y - vocabulary.max.y,
        "the vocabulary sits nearer the interval row than the hotkey row: not that group's footnote"
    );
}

#[test]
fn settings_banner_line_keeps_its_height_when_empty() {
    let (mut harness, persisted) = settings_harness(cfg("Alt+J", 7.0, false));
    let quiet_save = rect_of(&harness, "Save");
    let quiet_labels = label_texts(&harness);

    type_into(&mut harness, 1, "x");
    harness.key_press(egui::Key::Enter);
    harness.run();
    assert!(
        persisted.lock().unwrap().is_empty(),
        "the refused save must not write"
    );
    let banner = harness
        .get_all_by_role(Role::Label)
        .find(|n| !quiet_labels.contains(&n.value().unwrap_or_default()))
        .expect("the refused save raised no banner")
        .rect();

    let save = rect_of(&harness, "Save");
    let last_row = input_rect(&harness, 5);
    assert!(
        banner.min.y >= last_row.max.y - 0.5 && banner.max.y <= save.min.y + 0.5,
        "the banner (y={}..{}) is not on its line between the last row (ending y={}) and the buttons (y={})",
        banner.min.y,
        banner.max.y,
        last_row.max.y,
        save.min.y
    );
    assert!(
        (save.min.y - quiet_save.min.y).abs() <= 0.5,
        "the buttons moved from y={} to y={} when the banner appeared: its line was not reserved while empty",
        quiet_save.min.y,
        save.min.y
    );
}

#[cfg(target_os = "macos")]
#[test]
fn settings_form_adds_up_to_the_window() {
    let (harness, _persisted) = settings_harness(cfg("Alt+J", 7.0, false));
    let mut right = f32::MIN;
    let mut bottom = f32::MIN;
    for role in [Role::TextInput, Role::Button, Role::CheckBox, Role::Label] {
        for node in harness.get_all_by_role(role) {
            right = right.max(node.rect().max.x);
            bottom = bottom.max(node.rect().max.y);
        }
    }
    let margin = f32::from(MARGIN);
    assert!(
        (bottom + margin - WINDOW_SIZE[1]).abs() <= 1.0,
        "the form ends at y={bottom}, one margin above {} — the window is {} tall",
        bottom + margin,
        WINDOW_SIZE[1]
    );
    assert!(
        (right + margin - WINDOW_SIZE[0]).abs() <= 1.0,
        "the form ends at x={right}, one margin short of {} — the window is {} wide",
        right + margin,
        WINDOW_SIZE[0]
    );
}

#[test]
fn settings_long_hotkey_token_stays_on_the_message_line() {
    let one_word: String = std::iter::repeat_n("abcdefghij", 15).collect();
    let many_words: String = std::iter::repeat_n("abcde ", 25).collect();
    for token in [one_word, many_words] {
        long_token_stays_on_the_message_line(&token);
    }
}

fn long_token_stays_on_the_message_line(token: &str) {
    let (mut harness, persisted) = settings_harness(cfg("Alt+J", 7.0, false));
    let quiet_save = rect_of(&harness, "Save");
    let quiet_cancel = rect_of(&harness, "Cancel");
    let quiet_labels = label_texts(&harness);

    type_into(&mut harness, 0, token);
    harness.key_press(egui::Key::Enter);
    harness.run();
    assert!(
        persisted.lock().unwrap().is_empty(),
        "the refused save must not write"
    );
    let banner = harness
        .get_all_by_role(Role::Label)
        .find(|n| !quiet_labels.contains(&n.value().unwrap_or_default()))
        .expect("the refused save raised no banner");
    let text = banner.value().unwrap_or_default();
    let banner = banner.rect();

    let save = rect_of(&harness, "Save");
    let cancel = rect_of(&harness, "Cancel");
    let last_row = input_rect(&harness, 5);
    assert_eq!(save, quiet_save, "Save moved when the banner appeared");
    assert_eq!(
        cancel, quiet_cancel,
        "Cancel moved when the banner appeared"
    );
    assert!(
        banner.min.y >= last_row.max.y - 0.5 && banner.max.y <= save.min.y + 0.5,
        "the banner (y={}..{}) overflows its line between the last row (ending y={}) and the buttons (y={}): {text:?}",
        banner.min.y,
        banner.max.y,
        last_row.max.y,
        save.min.y
    );
    let margin = f32::from(MARGIN);
    assert!(
        banner.min.x >= margin - 0.5 && banner.max.x <= WINDOW_SIZE[0] - margin + 0.5,
        "the banner (x={}..{}) reaches past the form's margins: {text:?}",
        banner.min.x,
        banner.max.x
    );
}

fn painted_alpha(harness: &Harness<'_, SettingsApp>, texts: &[&str]) -> Vec<Option<f32>> {
    fn walk(shape: &egui::Shape, found: &mut Vec<(String, f32)>) {
        match shape {
            egui::Shape::Text(t) => {
                let glyph = t
                    .galley
                    .rows
                    .iter()
                    .flat_map(|r| r.row.visuals.mesh.vertices.first())
                    .map(|v| v.color)
                    .find(|c| *c != egui::Color32::PLACEHOLDER);
                let color = t.override_text_color.or(glyph).unwrap_or(t.fallback_color);
                found.push((t.galley.text().to_string(), f32::from(color.a()) / 255.0));
            }
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| walk(s, found)),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for clipped in &harness.output().shapes {
        walk(&clipped.shape, &mut found);
    }
    texts
        .iter()
        .map(|want| found.iter().find(|(t, _)| t == want).map(|(_, a)| *a))
        .collect()
}

#[test]
fn settings_dims_the_sprite_rows_while_the_sprite_is_off() {
    const TEXTS: [&str; 6] = [
        "Sprite",
        "PNG",
        "/art/s.png",
        "Position (pt)",
        "-41",
        "Card width",
    ];
    let config = |enabled: bool| {
        let mut config = cfg("Alt+J", 7.0, false);
        config.sprite.enabled = enabled;
        config.sprite.path = "/art/s.png".to_string();
        config.sprite.x = -41;
        config
    };
    let (on, _) = settings_harness(config(true));
    let (mut off, persisted) = settings_harness(config(false));
    let on_alpha = painted_alpha(&on, &TEXTS);
    let off_alpha = painted_alpha(&off, &TEXTS);
    assert!(
        on_alpha.iter().chain(&off_alpha).all(Option::is_some),
        "every text painted: on {on_alpha:?}, off {off_alpha:?}"
    );

    let dim = egui::Visuals::dark().disabled_alpha();
    let ratios: Vec<f32> = on_alpha
        .iter()
        .zip(&off_alpha)
        .map(|(on, off)| off.unwrap() / on.unwrap())
        .collect();
    let want = [1.0, dim, dim, dim, dim, dim];
    assert!(
        ratios.iter().zip(want).all(|(r, w)| (r - w).abs() < 0.02),
        "off/on alpha of {TEXTS:?}: {ratios:?}, want {want:?}"
    );

    type_into(&mut off, 2, "x");
    off.get_by_label("Save").click();
    off.run();
    let saved = persisted.lock().unwrap().last().map(|c| c.sprite.clone());
    assert!(
        saved
            .as_ref()
            .is_some_and(|s| !s.enabled && s.path.len() == "/art/s.png".len() + 1),
        "a dimmed field is still edited and saved: {saved:?}"
    );
}

#[derive(Default)]
struct UpdateCalls {
    checks: usize,
    installs: Vec<String>,
    opened: Vec<String>,
}

type Shared<T> = std::sync::Arc<std::sync::Mutex<T>>;

fn a_release(tag: &str) -> Release {
    Release {
        tag_name: tag.into(),
        html_url: format!("https://github.com/Wizard1209/nudge/releases/tag/{tag}"),
        assets: vec![],
    }
}

fn update_harness<'a>(
    answer: Result<Release, String>,
    installed: Result<(), String>,
    action: Action,
) -> (Harness<'a, SettingsApp>, Shared<UpdateCalls>) {
    let calls: Shared<UpdateCalls> = Default::default();
    let (c, i, o) = (calls.clone(), calls.clone(), calls.clone());
    let updates = UpdateHost {
        current: "0.2.2".into(),
        action,
        check: std::sync::Arc::new(move || {
            c.lock().unwrap().checks += 1;
            answer.clone()
        }),
        install: std::sync::Arc::new(move |r: &Release| {
            i.lock().unwrap().installs.push(r.tag_name.clone());
            installed.clone()
        }),
        open_page: Box::new(move |url: &str| o.lock().unwrap().opened.push(url.into())),
    };
    let provider: Box<dyn AutostartProvider> = Box::new(RecordingProvider::new(false));
    let mut harness = Harness::builder()
        .with_size(WINDOW_SIZE)
        .build_eframe(move |cc| {
            SettingsApp::new(
                cc,
                cfg("Alt+J", 7.0, false),
                SettingsHost {
                    updates,
                    ..SettingsHost::headless(provider)
                },
            )
        });
    harness.run();
    (harness, calls)
}

fn settle_on(harness: &mut Harness<'_, SettingsApp>, text: &str) {
    for _ in 0..200 {
        harness.step();
        if harness.query_by_label(text).is_some() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("{text:?} never appeared");
}

#[test]
fn settings_update_row_names_the_version_and_asks_nothing_until_pressed() {
    let (mut harness, calls) = update_harness(Ok(a_release("v0.3.0")), Ok(()), Action::Install);
    for _ in 0..5 {
        harness.step();
    }
    assert!(harness.query_by_label("Nudge 0.2.2").is_some());
    assert!(harness.query_by_label(update::FOOTNOTE).is_some());
    assert!(harness.query_by_label("Check for updates").is_some());
    assert_eq!(
        calls.lock().unwrap().checks,
        0,
        "opening the window asks nothing"
    );
}

#[test]
fn settings_update_check_offers_update_and_update_installs() {
    let (mut harness, calls) = update_harness(Ok(a_release("v0.3.0")), Ok(()), Action::Install);
    harness.get_by_label("Check for updates").click();
    settle_on(&mut harness, "0.3.0 available");
    assert_eq!(calls.lock().unwrap().checks, 1);

    harness.get_by_label("Update").click();
    settle_on(&mut harness, "Restarting…");
    assert_eq!(calls.lock().unwrap().installs, ["v0.3.0"]);
}

#[test]
fn settings_update_on_macos_opens_the_release_page() {
    let (mut harness, calls) = update_harness(Ok(a_release("v0.3.0")), Ok(()), Action::OpenPage);
    harness.get_by_label("Check for updates").click();
    settle_on(&mut harness, "0.3.0 available");
    harness.get_by_label("Open release page").click();
    harness.run();
    let calls = calls.lock().unwrap();
    assert_eq!(
        calls.opened,
        ["https://github.com/Wizard1209/nudge/releases/tag/v0.3.0"]
    );
    assert!(calls.installs.is_empty());
}

#[test]
fn settings_update_says_up_to_date_and_failures_in_the_row() {
    let (mut harness, _) = update_harness(Ok(a_release("v0.2.2")), Ok(()), Action::Install);
    harness.get_by_label("Check for updates").click();
    settle_on(&mut harness, "Up to date");

    let (mut harness, _) = update_harness(
        Err("Offline: could not reach GitHub".into()),
        Ok(()),
        Action::Install,
    );
    harness.get_by_label("Check for updates").click();
    settle_on(&mut harness, "Offline: could not reach GitHub");

    let (mut harness, _) = update_harness(
        Ok(a_release("v0.3.0")),
        Err("Download does not match its checksum".into()),
        Action::Install,
    );
    harness.get_by_label("Check for updates").click();
    settle_on(&mut harness, "0.3.0 available");
    harness.get_by_label("Update").click();
    settle_on(&mut harness, "Download does not match its checksum");
}
