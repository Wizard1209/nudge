//! Settings window — opened from the tray menu as a separate process.
//!
//! Two layers, deliberately split:
//!
//! - [`SettingsForm`] is the **pure state machine**: it holds the three field
//!   strings + bool, parses the interval, tracks dirty state, and emits a
//!   `Config` on Save. No egui, no I/O. This is what's unit-tested.
//!
//! - [`SettingsApp`] is the eframe shell that renders rows, calls `persist`
//!   on Save, and routes the autostart checkbox through
//!   [`crate::autostart::apply_autostart`] so the OS-confirms-first rule
//!   holds. The shell injects the persistence closure + provider trait
//!   object so the same code drives the native (file + registry) and the
//!   WASM (localStorage + fake provider) builds.
//!
//! The render layer renders **rows by hand** rather than going through a
//! `Setting` trait — three heterogeneous widgets don't justify the
//! abstraction yet, and inline rendering keeps the spec → screen mapping
//! direct.

use crate::autostart::{AutostartError, AutostartProvider, apply_autostart};
use crate::config::{Config, ConfigError};
use crate::hotkey::{self, Hotkey};

/// CLI flag that boots `nudge.exe` into the settings UI instead of the
/// popup. Single source of truth so the tray dispatch (spawn) and the main
/// dispatch (recognise) can't drift.
pub const SETTINGS_ARG: &str = "--settings";

/// Whether `--settings` appears anywhere in `args`. Used by the early
/// dispatch in `main` to route into the settings UI before the popup
/// codepath kicks in. Returns true on the first match; the rest of the args
/// are still available to the normal config-arg parser (`--config <path>`).
pub fn parse_settings_arg<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter().any(|a| a.as_ref() == SETTINGS_ARG)
}

/// The pure (no-eframe, no-I/O) settings form state. The render shell holds
/// one of these and asks it questions; the form never touches the disk.
#[derive(Debug, Clone)]
pub struct SettingsForm {
    /// Hotkey label as a free-text string. v3 has no recorder widget — the
    /// user types the label; we validate via [`crate::hotkey::parse`] only
    /// on Save so partially-typed values don't flicker errors.
    pub hotkey: String,
    /// Default interval (minutes) as the literal field text. Parsed via
    /// [`parse_interval_minutes`] on Save.
    pub interval_text: String,
    /// Mirrors the persisted autostart bit. The checkbox writes through this
    /// only AFTER `apply_autostart` confirms the OS change — never optimistically.
    pub autostart: bool,
    /// Snapshot of the loaded config so we can detect a dirty form without
    /// re-parsing. `is_dirty` compares against this.
    original: Config,
}

impl SettingsForm {
    /// Construct from a loaded config — every field starts in sync with disk.
    pub fn from_config(cfg: Config) -> Self {
        Self {
            hotkey: cfg.hotkey.clone(),
            interval_text: format_interval(cfg.default_interval_minutes),
            autostart: cfg.autostart,
            original: cfg,
        }
    }

    /// Parse `interval_text` into a finite-positive f64, with the same
    /// forgiveness as [`Config::resolved_interval_minutes`].
    pub fn parsed_interval(&self) -> Result<f64, IntervalParseError> {
        parse_interval_minutes(&self.interval_text)
    }

    /// Whether any field differs from the original loaded config. Test-only
    /// today (it's the canonical assertion that `mark_clean` reset the
    /// baseline); a future iteration may also surface a "discard changes?"
    /// confirmation, so we keep the API + its tests.
    #[allow(dead_code)]
    pub fn is_dirty(&self) -> bool {
        self.hotkey != self.original.hotkey
            || self.interval_text != format_interval(self.original.default_interval_minutes)
            || self.autostart != self.original.autostart
    }

    /// Build a `Config` from the current form, validating the interval. The
    /// `autostart` field is taken from the form's bool (which the caller
    /// keeps in sync with the OS via `apply_autostart`). On a successful
    /// save, the caller should call `mark_clean` so subsequent dirty checks
    /// don't keep tripping.
    pub fn to_config(&self) -> Result<Config, SettingsValidationError> {
        let interval = self
            .parsed_interval()
            .map_err(SettingsValidationError::Interval)?;
        Ok(Config {
            hotkey: self.hotkey.trim().to_string(),
            default_interval_minutes: interval,
            autostart: self.autostart,
        })
    }

    /// Mark the form's current state as the new baseline — called after a
    /// successful Save so `is_dirty` reflects "diff against last save".
    pub fn mark_clean(&mut self) {
        self.original = Config {
            hotkey: self.hotkey.trim().to_string(),
            default_interval_minutes: self.parsed_interval().unwrap_or(self.original.default_interval_minutes),
            autostart: self.autostart,
        };
        // Rehydrate text so trimming-on-save sticks visibly.
        self.hotkey = self.original.hotkey.clone();
        self.interval_text = format_interval(self.original.default_interval_minutes);
    }

    /// Drive the autostart checkbox through the transactional rule. On
    /// success, the form's `autostart` bool is flipped to `desired`; on
    /// failure, the bool is left as it was, and the caller surfaces the
    /// error string. `persist` is the same persistence closure the form's
    /// Save uses (native = file write, WASM = localStorage).
    pub fn toggle_autostart<F>(
        &mut self,
        provider: &dyn AutostartProvider,
        desired: bool,
        persist: F,
    ) -> Result<(), AutostartError>
    where
        F: FnOnce(&Config) -> Result<(), ConfigError>,
    {
        // The transactional rule mutates a Config-shaped buffer. We feed it
        // a snapshot of "current persisted view" (interval + hotkey from the
        // original baseline, autostart from the form's current bit) so a
        // successful toggle persists ONLY the autostart change — the other
        // fields aren't half-saved without the user clicking Save.
        let mut staged = Config {
            hotkey: self.original.hotkey.clone(),
            default_interval_minutes: self.original.default_interval_minutes,
            autostart: self.autostart,
        };
        apply_autostart(provider, &mut staged, desired, persist)?;
        // Provider + persist both succeeded; reflect into the form + baseline.
        self.autostart = desired;
        self.original.autostart = desired;
        Ok(())
    }
}

/// Validation failure modes for [`SettingsForm::to_config`]. Today only the
/// interval can fail; the variant carries the underlying parse error so the
/// banner can show the offending input verbatim.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingsValidationError {
    Interval(IntervalParseError),
}

impl std::fmt::Display for SettingsValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingsValidationError::Interval(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SettingsValidationError {}

/// Parse error for the default-interval field — mirrors the spec rule from
/// [`Config::resolved_interval_minutes`]: must be a finite, strictly positive
/// number. Whitespace tolerated.
#[derive(Debug, Clone, PartialEq)]
pub struct IntervalParseError {
    pub input: String,
}

impl std::fmt::Display for IntervalParseError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Interval must be a positive number of minutes (got: \"{}\")",
            self.input
        )
    }
}

impl std::error::Error for IntervalParseError {}

/// Parse the literal interval text exactly the way the main app's
/// `Config::resolved_interval_minutes` does: finite, > 0. Unlike the
/// popup's interval parser, we DON'T fall back silently on empty / unparseable —
/// the settings UI is the place to surface the error, not to paper over it.
pub fn parse_interval_minutes(text: &str) -> Result<f64, IntervalParseError> {
    let trimmed = text.trim();
    let n: f64 = trimmed.parse().map_err(|_| IntervalParseError {
        input: trimmed.to_string(),
    })?;
    if !n.is_finite() || n <= 0.0 {
        return Err(IntervalParseError {
            input: trimmed.to_string(),
        });
    }
    Ok(n)
}

/// Render an f64 as the text we put in the interval field. Whole numbers
/// print without a trailing `.0`; fractions keep their actual value.
fn format_interval(n: f64) -> String {
    if n.fract() == 0.0 && n.is_finite() {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

// =============================================================================
// Recorder decision logic — pure, no egui frame, fully unit-testable.
// =============================================================================

/// What the per-frame recorder loop wants the shell to do, given the current
/// egui input snapshot. Split out from the eframe shell so the policy
/// (cancel-on-bare-Esc, capture-first-supported-key, hint-on-unsupported)
/// can be tested without spinning a real `Context`.
#[derive(Debug, Clone, PartialEq)]
pub enum CaptureOutcome {
    /// A supported combo was pressed — write `format(hk)` into the form and
    /// leave recording mode.
    Captured(Hotkey),
    /// A non-modifier key was pressed, but our supported set rejects it —
    /// stay in recording mode and show a hint.
    Unsupported,
    /// User pressed Escape with no modifiers — cancel recording (restore
    /// the prior hotkey, leave recording mode). With modifiers held, Esc
    /// is treated as a real combo (Ctrl+Shift+Esc et al. are legal).
    CancelRequested,
    /// No actionable input this frame; keep waiting.
    KeepWaiting,
}

/// Per-frame recorder decision. `keys` is the snapshot of non-modifier keys
/// reported by egui as currently down (we ignore modifier-only keys because
/// egui doesn't surface raw Ctrl/Alt/Shift as `Key` variants in the first
/// place — see `hotkey_from_egui` notes).
///
/// Policy:
/// 1. Bare Escape (no modifiers held) → `CancelRequested`. With modifiers,
///    Escape passes through to the normal capture path (so Ctrl+Esc records).
/// 2. First non-modifier key in `keys` that maps via `hotkey_from_egui` →
///    `Captured`.
/// 3. First non-modifier key in `keys` that DOESN'T map → `Unsupported`.
/// 4. No keys → `KeepWaiting`.
pub fn decide_capture(
    modifiers: eframe::egui::Modifiers,
    keys: &[eframe::egui::Key],
) -> CaptureOutcome {
    use eframe::egui::Key;

    let Some(first_key) = keys.first().copied() else {
        return CaptureOutcome::KeepWaiting;
    };

    // Bare Escape cancels. With any modifier held, Esc is just another key.
    let no_modifiers = !(modifiers.ctrl
        || modifiers.alt
        || modifiers.shift
        || modifiers.mac_cmd
        || modifiers.command);
    if first_key == Key::Escape && no_modifiers {
        return CaptureOutcome::CancelRequested;
    }

    match hotkey::hotkey_from_egui(modifiers, first_key) {
        Some(hk) => CaptureOutcome::Captured(hk),
        None => CaptureOutcome::Unsupported,
    }
}

// =============================================================================
// eframe shell — render & event-loop layer. Cross-platform so it compiles on
// native AND wasm32; the *callers* differ (native main spawns it as a
// dedicated process, wasm boots it via the URL query branch in lib.rs).
// =============================================================================

/// Persistence closure: native writes to disk, WASM writes to localStorage.
/// Boxed so the SettingsApp can hold one across the run loop without a
/// generic parameter on the App impl (eframe::App is dyn-friendly that way).
pub type PersistFn = Box<dyn FnMut(&Config) -> Result<(), ConfigError> + Send>;

/// The settings window as an eframe::App. Owns the form state, the provider,
/// and the persistence closure; renders three rows + Save/Cancel + an error
/// banner.
pub struct SettingsApp {
    form: SettingsForm,
    provider: Box<dyn AutostartProvider>,
    persist: PersistFn,
    /// Surfaced to the user above the buttons. Cleared on the next interaction.
    banner: Option<String>,
    /// When set, the next frame ends the eframe loop. On native this exits
    /// the dedicated settings process; on WASM it's a no-op (no process).
    quit_requested: bool,
    /// True while the hotkey row is in capture mode. The TextEdit is hidden,
    /// the row shows a "Press a combo…" prompt, and per-frame input polling
    /// drives `decide_capture` to fill in the field.
    recording_hotkey: bool,
    /// Snapshot of the hotkey label taken when recording started — used by
    /// the cancel branch to restore the pre-recording value. `None` when
    /// we aren't recording.
    hotkey_pre_record: Option<String>,
    /// Hint shown below the row when the last captured key was unsupported.
    /// Cleared on entering / leaving recording mode.
    recording_hint: Option<&'static str>,
}

impl SettingsApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        config: Config,
        provider: Box<dyn AutostartProvider>,
        persist: PersistFn,
    ) -> Self {
        // Default eframe visuals — settings is NOT the spotlight. Opaque
        // panel, normal chrome. We still pin dark mode so the look matches
        // the popup. set_theme comes first: set_visuals only writes into
        // the active theme's style, and the default ThemePreference::System
        // switches to the OS theme after construction — on a light-themed
        // OS the window would silently render stock light visuals (see the
        // same pin in NudgeApp::new).
        cc.egui_ctx.set_theme(eframe::egui::ThemePreference::Dark);
        cc.egui_ctx.set_visuals(eframe::egui::Visuals::dark());
        Self {
            form: SettingsForm::from_config(config),
            provider,
            persist,
            banner: None,
            quit_requested: false,
            recording_hotkey: false,
            hotkey_pre_record: None,
            recording_hint: None,
        }
    }

    /// Enter capture mode. Snapshot the current label so a Cancel/Esc can
    /// restore it; clear any stale hint from a previous session.
    fn start_recording(&mut self) {
        self.hotkey_pre_record = Some(self.form.hotkey.clone());
        self.recording_hotkey = true;
        self.recording_hint = None;
    }

    /// Leave capture mode and restore the pre-recording label. Used for both
    /// the explicit Cancel button and the bare-Esc keystroke.
    fn cancel_recording(&mut self) {
        if let Some(prev) = self.hotkey_pre_record.take() {
            self.form.hotkey = prev;
        }
        self.recording_hotkey = false;
        self.recording_hint = None;
    }

    /// Save path: validate, persist, update baseline. On error → banner.
    fn save(&mut self) {
        match self.form.to_config() {
            Ok(cfg) => match (self.persist)(&cfg) {
                Ok(()) => {
                    self.form.mark_clean();
                    self.banner = Some("Saved".to_string());
                }
                Err(e) => self.banner = Some(format!("Save failed: {e}")),
            },
            Err(e) => self.banner = Some(e.to_string()),
        }
    }

    /// Autostart checkbox: route through apply_autostart so the system
    /// confirms before the form bool flips.
    fn apply_autostart_toggle(&mut self, desired: bool) {
        // Borrow split: pull the persist closure out by reference. We need
        // `&mut self.persist` inside the closure, so we shadow it with a
        // local FnOnce wrapper.
        let persist = &mut self.persist;
        let result = self.form.toggle_autostart(
            self.provider.as_ref(),
            desired,
            |cfg: &Config| persist(cfg),
        );
        match result {
            Ok(()) => self.banner = Some(if desired {
                "Autostart enabled".to_string()
            } else {
                "Autostart disabled".to_string()
            }),
            Err(e) => self.banner = Some(format!("{e}")),
        }
    }
}

impl eframe::App for SettingsApp {
    fn ui(&mut self, root_ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        use eframe::egui;

        let ctx_owned = root_ui.ctx().clone();
        let ctx = &ctx_owned;

        // Esc closes the window — unless we're recording a hotkey, in which
        // case the recording loop owns Esc as a cancel gesture. Without this
        // guard, hitting Esc while recording would close the entire settings
        // window before the recorder could honour the user's intent to bail.
        if !self.recording_hotkey && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.quit_requested = true;
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            self.quit_requested = true;
        }

        let ui = root_ui;
        ui.heading("Settings");
        ui.add_space(8.0);

        // Row 1: hotkey. Two modes — the plain TextEdit + "Record" button when
        // idle, and a "Press a combo…" prompt + Cancel button while
        // recording. Recording captures the first supported combo through
        // `decide_capture` and stuffs `format(hk)` back into the form's
        // hotkey string; Save still does the persistence.
        ui.horizontal(|ui| {
            ui.label("Global hotkey:");
            if self.recording_hotkey {
                ui.label(egui::RichText::new("Press a combo…").italics());
                if ui.button("Cancel").clicked() {
                    self.cancel_recording();
                }
            } else {
                ui.add(
                    egui::TextEdit::singleline(&mut self.form.hotkey)
                        .id(egui::Id::new("settings_hotkey"))
                        .desired_width(220.0)
                        .hint_text("Ctrl+Shift+Space"),
                );
                if ui.button("Record").clicked() {
                    self.start_recording();
                }
            }
        });
        if self.recording_hotkey {
            if let Some(hint) = self.recording_hint {
                ui.colored_label(egui::Color32::LIGHT_RED, hint);
            }
        }
        ui.add_space(6.0);

        // Per-frame recorder polling: while in recording mode, snapshot the
        // egui input and ask `decide_capture` what to do.
        if self.recording_hotkey {
            let (mods, keys) = ctx.input(|i| {
                let keys: Vec<egui::Key> = i.keys_down.iter().copied().collect();
                (i.modifiers, keys)
            });
            match decide_capture(mods, &keys) {
                CaptureOutcome::KeepWaiting => {}
                CaptureOutcome::CancelRequested => self.cancel_recording(),
                CaptureOutcome::Captured(hk) => {
                    self.form.hotkey = hotkey::format(&hk);
                    self.recording_hotkey = false;
                    self.hotkey_pre_record = None;
                    self.recording_hint = None;
                }
                CaptureOutcome::Unsupported => {
                    self.recording_hint = Some("Unsupported key, try another");
                }
            }
            // Keep repainting while recording so the next input frame arrives
            // promptly (eframe is otherwise lazy when no widget asks for it).
            ctx.request_repaint();
        }

        // Row 2: default interval
        ui.horizontal(|ui| {
            ui.label("Default interval (min):");
            ui.add(
                egui::TextEdit::singleline(&mut self.form.interval_text)
                    .id(egui::Id::new("settings_interval"))
                    .desired_width(120.0)
                    .hint_text("10"),
            );
        });
        ui.add_space(6.0);

        // Row 3: autostart — immediate-on-toggle through the transactional rule.
        ui.horizontal(|ui| {
            let mut autostart = self.form.autostart;
            let cb = ui.checkbox(&mut autostart, "Launch with Windows");
            if cb.changed() {
                self.apply_autostart_toggle(autostart);
            }
        });
        ui.add_space(12.0);

        // Error / status banner
        if let Some(msg) = &self.banner {
            ui.colored_label(egui::Color32::LIGHT_YELLOW, msg);
            ui.add_space(8.0);
        }

        ui.horizontal(|ui| {
            if ui.button("Save").clicked() {
                self.save();
            }
            if ui.button("Cancel").clicked() {
                self.quit_requested = true;
            }
        });

        if self.quit_requested {
            #[cfg(not(target_arch = "wasm32"))]
            ctx.send_viewport_cmd(eframe::egui::ViewportCommand::Close);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::autostart::FakeProvider;

    fn cfg_with(hotkey: &str, mins: f64, autostart: bool) -> Config {
        Config {
            hotkey: hotkey.to_string(),
            default_interval_minutes: mins,
            autostart,
        }
    }

    #[test]
    fn from_config_populates_fields() {
        let f = SettingsForm::from_config(cfg_with("Alt+J", 7.0, true));
        assert_eq!(f.hotkey, "Alt+J");
        assert_eq!(f.interval_text, "7");
        assert!(f.autostart);
        assert!(!f.is_dirty(), "freshly-loaded form is clean");
    }

    #[test]
    fn fractional_interval_renders_without_trailing_zero_noise() {
        // 0.5 → "0.5", not "0.5000000".
        let f = SettingsForm::from_config(cfg_with("Ctrl+Shift+Space", 0.5, false));
        assert_eq!(f.interval_text, "0.5");
    }

    #[test]
    fn editing_any_field_marks_dirty() {
        let original = cfg_with("Alt+J", 10.0, false);
        let mut f = SettingsForm::from_config(original);
        assert!(!f.is_dirty());

        f.hotkey = "Ctrl+F1".to_string();
        assert!(f.is_dirty(), "hotkey edit -> dirty");

        f.hotkey = "Alt+J".to_string();
        assert!(!f.is_dirty(), "reverted -> clean");

        f.interval_text = "5".to_string();
        assert!(f.is_dirty(), "interval edit -> dirty");

        f.interval_text = "10".to_string();
        assert!(!f.is_dirty(), "reverted -> clean");

        f.autostart = true;
        assert!(f.is_dirty(), "autostart edit -> dirty");
    }

    #[test]
    fn parse_interval_minutes_accepts_positive_finite() {
        assert_eq!(parse_interval_minutes("10").unwrap(), 10.0);
        assert_eq!(parse_interval_minutes("  7.5  ").unwrap(), 7.5);
        assert_eq!(parse_interval_minutes("0.1").unwrap(), 0.1);
    }

    #[test]
    fn parse_interval_minutes_rejects_garbage() {
        // Unlike the popup parser, the settings field is the user's
        // configured value; we MUST surface garbage, never silently fall back.
        for bad in ["", "abc", "0", "-5", "NaN", "Infinity"] {
            assert!(
                parse_interval_minutes(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn to_config_propagates_interval_error() {
        let mut f = SettingsForm::from_config(Config::default());
        f.interval_text = "garbage".to_string();
        let err = f.to_config().unwrap_err();
        assert!(matches!(err, SettingsValidationError::Interval(_)));
    }

    #[test]
    fn to_config_trims_hotkey() {
        // Leading/trailing whitespace in the hotkey label is user noise —
        // strip it before persisting so the parser sees a clean label.
        let mut f = SettingsForm::from_config(Config::default());
        f.hotkey = "  Alt+J  ".to_string();
        let cfg = f.to_config().unwrap();
        assert_eq!(cfg.hotkey, "Alt+J");
    }

    #[test]
    fn mark_clean_resets_dirty_after_edit() {
        let mut f = SettingsForm::from_config(Config::default());
        f.interval_text = "5".to_string();
        assert!(f.is_dirty());
        f.mark_clean();
        assert!(!f.is_dirty(), "mark_clean makes the current state the new baseline");
    }

    #[test]
    fn toggle_autostart_success_flips_form_and_persists() {
        // Provider OK + persist OK: form bool flips, baseline tracks it,
        // persisted config carries the new value.
        let provider = FakeProvider::new(false);
        let mut f = SettingsForm::from_config(Config::default());
        let saved = std::cell::RefCell::new(None);

        f.toggle_autostart(&provider, true, |c| {
            *saved.borrow_mut() = Some(c.clone());
            Ok(())
        })
        .unwrap();

        assert!(f.autostart, "form bool flipped");
        assert!(provider.is_enabled().unwrap(), "OS state actually enabled");
        assert_eq!(
            saved.borrow().as_ref().map(|c| c.autostart),
            Some(true),
            "persist saw autostart=true"
        );
        assert!(!f.is_dirty(), "baseline updated, so still clean");
    }

    #[test]
    fn toggle_autostart_backend_failure_leaves_form_untouched() {
        // Registry fails: the form bool MUST NOT flip — otherwise the UI
        // would lie about the OS state.
        let provider = FakeProvider::failing_enable();
        let mut f = SettingsForm::from_config(Config::default());
        let saved = std::cell::RefCell::new(None);

        let err = f
            .toggle_autostart(&provider, true, |c| {
                *saved.borrow_mut() = Some(c.clone());
                Ok(())
            })
            .unwrap_err();

        assert!(matches!(err, AutostartError::Backend(_)));
        assert!(!f.autostart, "form bool not flipped on backend failure");
        assert!(saved.borrow().is_none(), "persist never called");
    }

    #[test]
    fn parse_settings_arg_recognises_the_flag() {
        // Present anywhere in the arg list → true; absent → false. We
        // intentionally don't require positional ordering since the tray
        // spawn passes only the flag and tests may add scaffolding args.
        assert!(parse_settings_arg(["--settings"]));
        assert!(parse_settings_arg(["nudge.exe", "--settings"]));
        assert!(parse_settings_arg(["--config", "/x.json", "--settings"]));
        assert!(!parse_settings_arg(Vec::<&str>::new()));
        assert!(!parse_settings_arg(["--config", "/x.json"]));
        assert!(
            !parse_settings_arg(["--settingsxxx"]),
            "must be an exact match — no prefix gimmicks"
        );
    }

    // ---- decide_capture (recorder policy) ----------------------------------

    use eframe::egui::{Key, Modifiers};

    #[test]
    fn decide_capture_no_keys_keeps_waiting() {
        assert_eq!(
            decide_capture(Modifiers::NONE, &[]),
            CaptureOutcome::KeepWaiting
        );
    }

    #[test]
    fn decide_capture_bare_escape_cancels() {
        // Bare Esc is the universal cancel — recording a literal Esc-only
        // hotkey would collide with the cancel gesture, so we forbid it. A
        // user who really wants Esc can use Ctrl+Esc / Shift+Esc.
        assert_eq!(
            decide_capture(Modifiers::NONE, &[Key::Escape]),
            CaptureOutcome::CancelRequested
        );
    }

    #[test]
    fn decide_capture_escape_with_modifier_captures() {
        // Ctrl+Esc passes through as a real combo. Same for Shift+Esc, etc.
        let out = decide_capture(Modifiers::CTRL, &[Key::Escape]);
        match out {
            CaptureOutcome::Captured(hk) => {
                assert_eq!(hk.modifiers, crate::hotkey::MOD_CTRL);
                assert_eq!(hk.key.as_str(), "ESCAPE");
            }
            other => panic!("expected Captured, got {other:?}"),
        }
    }

    #[test]
    fn decide_capture_supported_key_captures() {
        let mods = Modifiers { ctrl: true, shift: true, ..Modifiers::NONE };
        let out = decide_capture(mods, &[Key::A]);
        match out {
            CaptureOutcome::Captured(hk) => {
                assert_eq!(crate::hotkey::format(&hk), "Ctrl+Shift+A");
            }
            other => panic!("expected Captured, got {other:?}"),
        }
    }

    #[test]
    fn decide_capture_unsupported_key_signals_hint() {
        // Home isn't in our allowlist (vk_for_key wouldn't recognise it).
        assert_eq!(
            decide_capture(Modifiers::CTRL, &[Key::Home]),
            CaptureOutcome::Unsupported
        );
    }

    #[test]
    fn decide_capture_uses_first_key() {
        // If two keys are down in the same frame (rare but possible during
        // chord release), we honour the first — keeps the policy deterministic.
        let out = decide_capture(Modifiers::CTRL, &[Key::A, Key::B]);
        match out {
            CaptureOutcome::Captured(hk) => assert_eq!(hk.key.as_str(), "A"),
            other => panic!("expected Captured(A), got {other:?}"),
        }
    }

    #[test]
    fn toggle_autostart_persist_failure_surfaces_but_os_still_changed() {
        // The OS change stuck and was confirmed; only the disk write failed.
        // form.autostart MUST NOT flip — otherwise the UI claims a state
        // the persisted config doesn't carry.
        let provider = FakeProvider::new(false);
        let mut f = SettingsForm::from_config(Config::default());

        let err = f
            .toggle_autostart(&provider, true, |_c| {
                Err(ConfigError::Io {
                    path: "x".to_string(),
                    detail: "disk full".to_string(),
                })
            })
            .unwrap_err();

        assert!(matches!(err, AutostartError::Persist(_)));
        assert!(provider.is_enabled().unwrap(), "OS change still happened");
        assert!(!f.autostart, "form bool not flipped on persist failure");
    }
}
