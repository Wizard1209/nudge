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
use nudge_web::recorder::{self, EdgeSource, PhysicalKey, PressEdge, Verdict};
use nudge_web::settings_app::{
    MARGIN, PersistFn, SettingsApp, SettingsForm, SpriteCheckFn, WINDOW_SIZE,
};

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

/// A PNG check that passes everything: no test here reads a file.
fn accept_any_sprite() -> SpriteCheckFn {
    Box::new(|_path: &std::path::Path| Ok(()))
}

/// The autostart checkbox, by its platform label — there are two checkboxes
/// in the window now, and the sprite's is the other one.
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
            provider,
            no_op_persist(),
            accept_any_sprite(),
        )
    });
    harness.run();

    // Hotkey row, then interval row, then the sprite's path, X, Y and card
    // width: all TextEdit::singleline with hint_text only, same unlabelled
    // shape as the spotlight card's fields (the row labels are separate
    // ui.label()s, not wired via .labelled_by()).
    let inputs: Vec<_> = harness.get_all_by_role(Role::TextInput).collect();
    assert_eq!(
        inputs.len(),
        6,
        "hotkey, interval, sprite path, X, Y, card width"
    );
    // The stored spelling is the cross-platform one; the field shows this
    // platform's — `⌥J` on macOS. Deriving the expectation the same way the
    // window does would assert nothing, so this states both forms.
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
        SettingsApp::new(
            cc,
            cfg("Alt+J", 7.0, false),
            provider,
            no_op_persist(),
            accept_any_sprite(),
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
        SettingsApp::new(
            cc,
            cfg("Alt+J", 7.0, false),
            boxed_provider,
            persist,
            accept_any_sprite(),
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

/// §9: an enabled sprite must pass the §10 PNG check or Save refuses it,
/// naming the reason on the window's message line and writing nothing.
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
        SettingsApp::new(cc, cfg("Alt+J", 7.0, false), provider, persist, refuse)
    });
    harness.run();

    // Type a path into the sprite row's third input, tick Sprite, Save.
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
    // §9 Saving: only "a save that reached the file says so".
    assert!(
        harness.query_by_label("Saved").is_none(),
        "a refused save must not say Saved"
    );
}

// -----------------------------------------------------------------------------
// §9 Saving — Return, the `Saved` note, and a Save that is live only while
// the form differs from the file.
// -----------------------------------------------------------------------------

/// A settings window over a recording persist closure: what Save wrote, in
/// order, so a case can assert on the write rather than on the window alone.
/// The harness is the window's own size, so what the layout cases measure is
/// the form as the window shows it.
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
            SettingsApp::new(cc, config, provider, persist, accept_any_sprite())
        });
    harness.run();
    (harness, persisted)
}

/// A key source with no platform behind it: edges are whatever a test queues,
/// every one of them refused by name, and `armed` is what the row last said.
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

/// [`settings_harness`] with a recorder that can arm.
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
                provider,
                persist,
                accept_any_sprite(),
                Some(Box::new(source)),
            )
        });
    harness.run();
    (harness, persisted)
}

/// Put the caret in the n-th text input and type into it.
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

/// §9 Saving: "Return anywhere in the form is `Save`: the same checks, the
/// same write, the same messages as the button."
#[test]
fn settings_return_in_a_field_saves() {
    let (mut harness, persisted) = settings_harness(cfg("Alt+J", 7.0, false));

    // The interval field: "7" becomes "75" (the caret lands at the end).
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

    // A sprite field: the same key, the same write.
    type_into(&mut harness, 3, "4");
    harness.key_press(egui::Key::Enter);
    harness.run();
    assert_eq!(
        persisted.lock().unwrap().last().map(|c| c.sprite.x),
        Some(4),
        "Return in the sprite X field did not write the form"
    );
}

/// §9 Saving: "the same checks, the same write, the same messages as the
/// button" — a Return that fails the checks shows the button's message and
/// writes nothing.
#[test]
fn settings_return_runs_the_same_checks_and_shows_the_same_message() {
    let (mut harness, persisted) = settings_harness(cfg("Alt+J", 7.0, false));

    type_into(&mut harness, 1, "x");
    harness.key_press(egui::Key::Enter);
    harness.run();

    assert!(
        harness
            .query_by_label("Interval must be a positive number of minutes (got: \"7x\")")
            .is_some(),
        "the message line does not carry the interval check's reason"
    );
    assert!(
        persisted.lock().unwrap().is_empty(),
        "a refused save from Return wrote something"
    );
    assert!(!says_saved(&harness), "a refused save must not say Saved");
}

/// §9 Saving: "`Save` is live only while the form differs from the file" —
/// so Return on a form that matches the file has nothing to do.
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

/// §9 Saving: "While the recorder is armed the key is the recorder's — a
/// press, not a save." The source swallows Return on a real desktop; here
/// egui sees it too, and the window must still not save.
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

    // The press edge the monitor would report, and the key egui would see.
    source.edges.lock().unwrap().push(PressEdge {
        key: PhysicalKey(36),
        modifiers: 0,
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

/// §9 Saving: "A focused button or checkbox keeps its own Return" — on the
/// autostart checkbox the key is the checkbox's click, and the edited
/// interval stays unsaved.
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

/// §9 Saving: "A save that reached the file says so — `Saved`, beside the
/// buttons — and the first edit after it takes the note away."
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

/// §9 Saving: "The note is the button's: an autostart click, committed on
/// its own, takes it away like any other edit and raises none."
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

/// §9 Saving: "`Save` is live only while the form differs from the file, so
/// a form that reads as unedited is one whose content is on disk."
#[test]
fn settings_save_is_greyed_while_the_form_matches_the_file() {
    let (mut harness, persisted) = settings_harness(cfg("Alt+J", 7.0, false));
    assert!(
        save_is_disabled(&harness),
        "a freshly-loaded form has nothing to save"
    );

    // "7" -> "75": differs from the file.
    type_into(&mut harness, 1, "5");
    assert!(!save_is_disabled(&harness), "an edit makes Save live");

    // Backspace over the "5": the stored value is back, so nothing differs.
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

    // A greyed button is not a button: a click on it writes nothing.
    harness.get_by_label("Save").click();
    harness.run();
    assert_eq!(persisted.lock().unwrap().len(), 1);
}

// -----------------------------------------------------------------------------
// §9 Window chrome and layout.
//
// One case per sentence of the subsection, against the Rust settings window;
// their Electron twins, on the same sentences, are
// electron/tests/native/settings.test.js.
// -----------------------------------------------------------------------------

use egui_kittest::kittest::By;

/// The autostart row's text on this platform (§9 Fields).
#[cfg(target_os = "macos")]
const AUTOSTART_ROW: &str = "Open at login";
#[cfg(not(target_os = "macos"))]
const AUTOSTART_ROW: &str = "Launch with Windows";

/// The rect of the widget labelled `label`, or a failure that names it.
fn rect_of(harness: &Harness<'_, SettingsApp>, label: &str) -> egui::Rect {
    harness
        .query_by_label(label)
        .unwrap_or_else(|| panic!("no widget labelled {label:?} in the settings window"))
        .rect()
}

/// The rect of the row text `text` itself — the label, never the control it
/// labels (kittest hides a labelling node from label queries by default).
fn row_text_rect(harness: &Harness<'_, SettingsApp>, text: &str) -> egui::Rect {
    harness
        .query_all(By::new().role(Role::Label).label(text).include_labels())
        .next()
        .unwrap_or_else(|| panic!("no row text {text:?} in the settings window"))
        .rect()
}

/// The n-th text input in draw order: hotkey, interval, PNG path, X, Y,
/// card width.
fn input_rect(harness: &Harness<'_, SettingsApp>, nth: usize) -> egui::Rect {
    harness
        .get_all_by_role(Role::TextInput)
        .nth(nth)
        .unwrap_or_else(|| panic!("fewer than {} text inputs", nth + 1))
        .rect()
}

/// The two checkboxes in draw order: autostart, sprite.
fn checkbox_rects(harness: &Harness<'_, SettingsApp>) -> Vec<egui::Rect> {
    let rects: Vec<egui::Rect> = harness
        .get_all_by_role(Role::CheckBox)
        .map(|n| n.rect())
        .collect();
    assert_eq!(rects.len(), 2, "two checkboxes: autostart and sprite");
    rects
}

/// The texts of every label on screen.
fn label_texts(harness: &Harness<'_, SettingsApp>) -> Vec<String> {
    harness
        .get_all_by_role(Role::Label)
        .filter_map(|n| n.value())
        .collect()
}

/// Whether all of `edges` lie within a point of each other.
fn one_edge(edges: &[(&str, f32)]) -> bool {
    let min = edges.iter().map(|e| e.1).fold(f32::MAX, f32::min);
    let max = edges.iter().map(|e| e.1).fold(f32::MIN, f32::max);
    max - min <= 1.0
}

/// §9: "Nothing inside the window repeats the title: the title bar carries
/// it."
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

/// §9: "`Cancel`, then `Save`, at the bottom right".
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

    // At the bottom right: nothing of the form ends further right or lower.
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
}

/// §9: "The `Saved` note sits to their left after a save."
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

/// §9: "In every row the label sits at the left and the control at the right
/// edge." Six rows, one left edge for their texts, one right edge for the
/// last control of each. The row texts are §9's Fields names; the PNG and
/// position rows read `PNG` and `Position (pt)` (§10: "labelled in points").
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

    // The last control of each row. The hotkey row ends with Record where a
    // recorder exists, the PNG row with Browse… where a file dialog does;
    // otherwise with the field itself.
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

/// §9: "A checkbox sits at the right of its row; the row's text is its label."
#[test]
fn settings_checkboxes_sit_at_the_right_of_their_rows() {
    let (harness, _persisted) = settings_harness(cfg("Alt+J", 7.0, false));
    let checkboxes = checkbox_rects(&harness);
    // The right edge every row's control ends at: the hotkey row's last one.
    // (Not Save's — the buttons sit at the window's margin, the rows inside
    // their boxes.)
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

/// §9: "The hotkey vocabulary is a footnote under its group."
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

/// §9: "The banner has its line above the buttons whether or not it has
/// anything to say, so the window never changes size."
#[test]
fn settings_banner_line_keeps_its_height_when_empty() {
    let (mut harness, persisted) = settings_harness(cfg("Alt+J", 7.0, false));
    let quiet_save = rect_of(&harness, "Save");
    let quiet_labels = label_texts(&harness);

    // "7" -> "7x": refused, so the banner has something to say.
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

/// §9: "The window is sized to the form" — "on the native build the content
/// is 560 × 537 pt". The form, laid out at that width, ends one margin above
/// the window's bottom and one margin short of its right edge: the number in
/// the spec is what the form adds up to, not a guess the window asks for.
/// The native build is the macOS one — the footnote's wrap, and so the sum,
/// is this platform's vocabulary in this platform's spelling.
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

/// §9: "The banner has its line above the buttons whether or not it has
/// anything to say, so the window never changes size." A refused hotkey
/// echoes what was typed, and what was typed can be anything: the message
/// has to stay on its line whatever it quotes, and the form has to keep its
/// shape under it — the buttons where they were, nothing past the margin.
/// Two shapes of anything: one word that no line can hold, which pushes
/// sideways, and a run of words, which wraps and pushes up.
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
