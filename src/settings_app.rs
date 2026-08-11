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
    ///
    /// A hotkey that parses is *shown* in this platform's spelling whatever
    /// the file says, so a config written by the Windows build, seeded by
    /// first-run, or edited by hand reads as `⌃⇧Space` on a Mac rather than
    /// `Ctrl+Shift+Space`. The baseline is normalised with it, so re-spelling
    /// alone does not make the form look edited.
    ///
    /// One that does *not* parse is shown verbatim: it is the only copy of
    /// what the user typed, and they need to see it to fix it.
    pub fn from_config(cfg: Config) -> Self {
        let mut cfg = cfg;
        if let Ok(hk) = hotkey::parse(&cfg.hotkey) {
            cfg.hotkey = hotkey::format_native(&hk);
        }
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

    /// The typed hotkey, parsed. The field accepts every spelling
    /// [`hotkey::parse`] does — `⌥⌘J`, `Alt+Cmd+J`, `alt+cmd+j` — so typing
    /// one is a first-class alternative to the recorder, not a fallback.
    pub fn parsed_hotkey(&self) -> Result<Hotkey, hotkey::ParseError> {
        hotkey::parse(&self.hotkey)
    }

    /// Build a `Config` from the current form, validating both editable
    /// fields. The `autostart` field is taken from the form's bool (which the
    /// caller keeps in sync with the OS via `apply_autostart`). On a
    /// successful save, the caller should call `mark_clean` so subsequent
    /// dirty checks don't keep tripping.
    pub fn to_config(&self) -> Result<Config, SettingsValidationError> {
        // Both fields are validated before either is written, so a bad
        // hotkey cannot leave a saved interval next to an unsaved combo.
        let interval = self
            .parsed_interval()
            .map_err(SettingsValidationError::Interval)?;
        let hotkey = self
            .parsed_hotkey()
            .map_err(SettingsValidationError::Hotkey)?;
        Ok(Config {
            // Canonical, not as-typed: the file gets one spelling per
            // combination, and the field is rehydrated with it on save so the
            // user sees what was actually stored.
            hotkey: hotkey::format_native(&hotkey),
            default_interval_minutes: interval,
            autostart: self.autostart,
        })
    }

    /// Mark the form's current state as the new baseline — called after a
    /// successful Save so `is_dirty` reflects "diff against last save".
    /// `saved` is the `Config` that actually reached the disk, not a
    /// re-derivation of it: the fields are rehydrated from it so what the
    /// user reads back is the stored text — the canonical hotkey spelling
    /// rather than however they typed it.
    pub fn mark_clean(&mut self, saved: Config) {
        self.hotkey = saved.hotkey.clone();
        self.interval_text = format_interval(saved.default_interval_minutes);
        self.autostart = saved.autostart;
        self.original = saved;
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

/// Validation failure modes for [`SettingsForm::to_config`]. Each variant
/// carries the underlying parse error so the banner can show the offending
/// input verbatim.
#[derive(Debug, Clone, PartialEq)]
pub enum SettingsValidationError {
    Interval(IntervalParseError),
    Hotkey(hotkey::ParseError),
}

impl std::fmt::Display for SettingsValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingsValidationError::Interval(e) => write!(f, "{e}"),
            // The parse error says what is wrong with the string; this adds
            // what a correct one looks like, because the banner is the only
            // place the accepted vocabulary is stated.
            SettingsValidationError::Hotkey(e) => write!(f, "{e}. {}", hotkey::VOCABULARY),
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
// The recorder — spec §9. A two-state machine over key *press edges*.
// =============================================================================

/// What a single key press means to a recorder that is armed.
///
/// There is no third input and no clock. Everything the recorder can be told
/// arrives as one press edge carrying its own modifiers, so this function is
/// the whole transition relation and it is total.
#[derive(Debug, Clone, PartialEq)]
pub enum Capture {
    /// A registrable combination — write it into the field and disarm.
    Took(Hotkey),
    /// Bare Escape. A chord is modifiers plus a key, so a lone Escape can
    /// never be one, which is what leaves the keystroke free to mean "back
    /// out". With any modifier held it is an ordinary key and `⌃⎋` records.
    Cancel,
    /// A key no backend can register. Stay armed and say so — disarming here
    /// would make a mistyped chord look like a recorded one.
    Refuse(&'static str),
}

/// Decide what one key press does to an armed recorder.
///
/// **Press edges, not the set of held keys.** egui exposes both; only this
/// one is trustworthy. Measured on a real macOS typing session (965 frames):
/// while a modifier was held, the key vanished from `keys_down` 25 times
/// mid-press, for 0.08–0.63 s at a time — but the modifiers were unchanged
/// across every one of those gaps. The key was re-arriving as fresh press
/// events. So a rule phrased over presses sees a clean, plentiful signal
/// exactly where a rule phrased over the held set sees a hole.
///
/// That is also why nothing here is timed. A duration rule needs a stable
/// "still held", which is the one thing this platform does not provide.
pub fn decide_capture(modifiers: eframe::egui::Modifiers, key: eframe::egui::Key) -> Capture {
    use eframe::egui::Key;

    let bare = !(modifiers.ctrl || modifiers.alt || modifiers.shift || modifiers.mac_cmd);
    if key == Key::Escape && bare {
        return Capture::Cancel;
    }
    match hotkey::hotkey_from_egui(modifiers, key) {
        Some(hk) => Capture::Took(hk),
        None => Capture::Refuse("Эту клавишу связать нельзя — попробуй другую"),
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
    /// `Some(previous label)` while the recorder is armed — the two states of
    /// the §9 machine, and the snapshot a cancel restores in one field.
    recording: Option<String>,
    /// Shown under the row after a key that cannot be bound.
    recording_hint: Option<&'static str>,
    /// When set, the next frame ends the eframe loop. On native this exits
    /// the dedicated settings process; on WASM it's a no-op (no process).
    quit_requested: bool,
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
        // The hotkey row is the only place in the app that renders symbols
        // outside egui's bundled faces, and on macOS every one of them
        // (`⌃⌥⇧⌘⎋⇥⌫↩`) is missing from them.
        #[cfg(not(target_arch = "wasm32"))]
        crate::platform::install_native_fonts(&cc.egui_ctx);
        Self {
            form: SettingsForm::from_config(config),
            provider,
            persist,
            banner: None,
            recording: None,
            recording_hint: None,
            quit_requested: false,
        }
    }

    /// Arm the recorder, snapshotting the label a cancel restores.
    ///
    /// Keyboard focus is surrendered first: while armed the recorder reads
    /// every press, and a TextEdit still holding focus would swallow them
    /// into the interval field instead.
    fn arm(&mut self, ctx: &eframe::egui::Context) {
        self.recording = Some(self.form.hotkey.clone());
        self.recording_hint = None;
        ctx.memory_mut(|m| m.surrender_focus(eframe::egui::Id::new("settings_interval")));
        ctx.memory_mut(|m| m.surrender_focus(eframe::egui::Id::new("settings_hotkey")));
    }

    /// Disarm. `keep` is the label to leave in the field: the captured one,
    /// or the snapshot when the user backed out.
    fn disarm(&mut self, keep: Option<String>) {
        if let Some(label) = keep {
            self.form.hotkey = label;
        }
        self.recording = None;
        self.recording_hint = None;
    }

    /// Save path: validate, persist, update baseline. On error → banner.
    fn save(&mut self) {
        match self.form.to_config() {
            Ok(cfg) => match (self.persist)(&cfg) {
                Ok(()) => {
                    self.form.mark_clean(cfg);
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
        let result = self
            .form
            .toggle_autostart(self.provider.as_ref(), desired, |cfg: &Config| persist(cfg));
        match result {
            Ok(()) => {
                self.banner = Some(if desired {
                    "Autostart enabled".to_string()
                } else {
                    "Autostart disabled".to_string()
                })
            }
            Err(e) => self.banner = Some(format!("{e}")),
        }
    }
}

impl eframe::App for SettingsApp {
    fn ui(&mut self, root_ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        use eframe::egui;

        let ctx_owned = root_ui.ctx().clone();
        let ctx = &ctx_owned;

        // Esc closes the window — except while the recorder is armed, where
        // it is that machine's cancel. Without this guard, backing out of a
        // recording would take the whole window with it.
        if self.recording.is_none() && ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.quit_requested = true;
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            self.quit_requested = true;
        }

        let ui = root_ui;
        ui.heading("Settings");
        ui.add_space(8.0);

        // Row 1: hotkey. Two states, exactly as spec §9 lays them out —
        // a text field plus `Запись`, or an armed prompt plus `Отмена`.
        //
        // The recorder reads *press edges* off this frame's event list. It
        // never looks at `keys_down`: that set drops a held key repeatedly on
        // macOS while a modifier is down, which is what made the previous
        // hold-to-commit recorder unable to accept anything at all.
        if self.recording.is_some() {
            let pressed: Option<(egui::Key, egui::Modifiers)> = ctx.input(|i| {
                i.events.iter().find_map(|e| match e {
                    // `repeat` is skipped so a key already down when the
                    // recorder armed cannot be captured by its autorepeat.
                    egui::Event::Key {
                        key,
                        pressed: true,
                        repeat: false,
                        modifiers,
                        ..
                    } => Some((*key, *modifiers)),
                    _ => None,
                })
            });
            if let Some((key, modifiers)) = pressed {
                if std::env::var_os("NUDGE_RECORDER_LOG").is_some() {
                    eprintln!("[rec] press key={key:?} mods={modifiers:?}");
                }
                match decide_capture(modifiers, key) {
                    Capture::Took(hk) => self.disarm(Some(hotkey::format_native(&hk))),
                    Capture::Cancel => {
                        let previous = self.recording.clone();
                        self.disarm(previous);
                    }
                    Capture::Refuse(hint) => self.recording_hint = Some(hint),
                }
            }
            ctx.request_repaint();
        }

        ui.horizontal(|ui| {
            ui.label("Global hotkey:");
            match self.recording.clone() {
                None => {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.form.hotkey)
                            .id(egui::Id::new("settings_hotkey"))
                            .desired_width(220.0)
                            .hint_text(hotkey::format_native(&hotkey::default_hotkey())),
                    );
                    if ui.button("Записать").clicked() {
                        self.arm(ctx);
                    }
                }
                Some(previous) => {
                    ui.add_sized(
                        [220.0, ui.spacing().interact_size.y],
                        egui::Label::new(egui::RichText::new("нажми комбинацию…").strong()),
                    );
                    if ui.button("Отмена").clicked() {
                        self.disarm(Some(previous));
                    }
                }
            }
        });
        if self.recording.is_some() {
            ui.label(
                egui::RichText::new("Модификаторы + одна клавиша. Esc — отмена.")
                    .small()
                    .weak(),
            );
        } else {
            ui.label(egui::RichText::new(hotkey::VOCABULARY).small().weak());
        }
        if let Some(hint) = self.recording_hint {
            ui.colored_label(egui::Color32::LIGHT_RED, hint);
        }
        ui.add_space(6.0);

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
            // Each OS has its own name for this and its own mechanism behind
            // it — a Run key on Windows, a LaunchAgent on macOS. Naming the
            // wrong one is not cosmetic: it is the label on the control that
            // decides whether the user believes the setting applies to them.
            #[cfg(target_os = "macos")]
            let label = "Open at login";
            #[cfg(not(target_os = "macos"))]
            let label = "Launch with Windows";
            let cb = ui.checkbox(&mut autostart, label);
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

    /// `Alt+J` in this platform's spelling — `⌥J` on macOS.
    fn native(label: &str) -> String {
        crate::hotkey::format_native(&crate::hotkey::parse(label).unwrap())
    }

    #[test]
    fn from_config_populates_fields() {
        let f = SettingsForm::from_config(cfg_with("Alt+J", 7.0, true));
        assert_eq!(f.hotkey, native("Alt+J"));
        assert_eq!(f.interval_text, "7");
        assert!(f.autostart);
        assert!(!f.is_dirty(), "freshly-loaded form is clean");
    }

    #[test]
    fn a_config_written_elsewhere_is_shown_in_this_platforms_spelling() {
        // Hand-edited files and configs seeded by the other platform's build
        // both arrive spelled the other way. Showing them verbatim would make
        // the field the one place in the window that is not native.
        let f = SettingsForm::from_config(cfg_with("  alt + j  ", 10.0, false));
        assert_eq!(f.hotkey, native("Alt+J"));
        assert!(
            !f.is_dirty(),
            "re-spelling on load is not an edit the user made"
        );
    }

    #[test]
    fn an_unparseable_stored_hotkey_is_shown_exactly_as_stored() {
        // It is the only copy of what the user typed, and the field is where
        // they have to fix it.
        let f = SettingsForm::from_config(cfg_with("Ctrl+Hoem", 10.0, false));
        assert_eq!(f.hotkey, "Ctrl+Hoem");
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

        f.hotkey = native("Ctrl+F1");
        assert!(f.is_dirty(), "hotkey edit -> dirty");

        f.hotkey = native("Alt+J");
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
    fn to_config_canonicalises_however_the_hotkey_was_typed() {
        // Whitespace, case and the choice of spelling are user noise. One
        // combination gets one representation on disk, whichever of the
        // accepted forms was typed or recorded.
        let canonical = crate::hotkey::format_native(&crate::hotkey::parse("Alt+J").unwrap());
        for typed in ["  Alt+J  ", "alt+j", "Option+J", "⌥J"] {
            let mut f = SettingsForm::from_config(Config::default());
            f.hotkey = typed.to_string();
            assert_eq!(
                f.to_config().unwrap().hotkey,
                canonical,
                "typed {typed:?} must persist as {canonical:?}"
            );
        }
    }

    #[test]
    fn a_hotkey_that_cannot_be_registered_blocks_the_save() {
        // The field is free text, so it is the only place an unregistrable
        // combination can enter the config. Letting it through means the
        // banner says "Saved" and the next launch has no hotkey.
        for typed in ["Ctrl+Hoem", "Ctrl+Shift", "", "   ", "Ctrl+A+B"] {
            let mut f = SettingsForm::from_config(Config::default());
            f.hotkey = typed.to_string();
            let err = f
                .to_config()
                .expect_err("{typed:?} must not reach the config");
            assert!(
                matches!(err, SettingsValidationError::Hotkey(_)),
                "{typed:?} produced {err:?}"
            );
        }
    }

    #[test]
    fn the_hotkey_error_names_both_the_fault_and_the_vocabulary() {
        // The banner is where a user learns what they may type; the parse
        // error alone says only what went wrong.
        let mut f = SettingsForm::from_config(Config::default());
        f.hotkey = "Ctrl+Shift".to_string();
        let shown = f.to_config().unwrap_err().to_string();
        assert!(shown.contains("no key"), "{shown}");
        assert!(shown.contains(crate::hotkey::VOCABULARY), "{shown}");
    }

    #[test]
    fn mark_clean_resets_dirty_after_edit() {
        let mut f = SettingsForm::from_config(Config::default());
        f.interval_text = "5".to_string();
        assert!(f.is_dirty());
        let saved = f.to_config().unwrap();
        f.mark_clean(saved);
        assert!(
            !f.is_dirty(),
            "mark_clean makes the current state the new baseline"
        );
    }

    #[test]
    fn after_a_save_the_field_shows_what_was_stored() {
        // The user types one of the accepted spellings; the file gets the
        // canonical one. If the field kept the typed text, the form would
        // read as dirty the moment it was saved, and Cancel would look like
        // it was discarding a change that had already happened.
        let mut f = SettingsForm::from_config(Config::default());
        f.hotkey = "option+cmd+j".to_string();
        let saved = f.to_config().unwrap();
        f.mark_clean(saved.clone());
        assert_eq!(f.hotkey, saved.hotkey);
        assert!(!f.is_dirty(), "a just-saved form is not dirty");
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

    // ---- the recorder (spec §9) --------------------------------------------

    use eframe::egui::{Key, Modifiers};

    fn ctrl_shift() -> Modifiers {
        Modifiers {
            ctrl: true,
            shift: true,
            ..Modifiers::NONE
        }
    }

    #[test]
    fn a_press_with_modifiers_is_taken_whole() {
        match decide_capture(ctrl_shift(), Key::A) {
            Capture::Took(hk) => assert_eq!(crate::hotkey::format(&hk), "Ctrl+Shift+A"),
            other => panic!("expected Took, got {other:?}"),
        }
    }

    #[test]
    fn bare_escape_cancels_but_escape_with_a_modifier_records() {
        assert_eq!(
            decide_capture(Modifiers::NONE, Key::Escape),
            Capture::Cancel
        );
        match decide_capture(Modifiers::CTRL, Key::Escape) {
            Capture::Took(hk) => assert_eq!(crate::hotkey::format(&hk), "Ctrl+Escape"),
            other => panic!("expected Took(Ctrl+Escape), got {other:?}"),
        }
    }

    #[test]
    fn a_lone_command_press_cannot_reach_the_recorder_at_all() {
        // Modifiers are not `Key` variants, so a chord of them produces no
        // press edge and the recorder simply keeps waiting. This is the case
        // that used to look like the recorder ignoring the user.
        assert!(
            crate::hotkey::hotkey_from_egui(
                Modifiers {
                    mac_cmd: true,
                    command: true,
                    alt: true,
                    ..Modifiers::NONE
                },
                Key::J,
            )
            .is_some(),
            "⌥⌘ plus a key is bindable — it is the chord alone that is not an event"
        );
    }

    #[test]
    fn a_key_no_backend_can_register_is_refused_without_disarming() {
        assert!(matches!(
            decide_capture(Modifiers::CTRL, Key::Home),
            Capture::Refuse(_)
        ));
    }

    #[test]
    fn every_capture_round_trips_through_the_field() {
        // The shell writes `format_native(taken)` into the text field and Save
        // hands that string to the config, so anything the recorder can
        // produce must parse back — otherwise a recorded hotkey is unreadable
        // at next launch.
        let cases = [
            (ctrl_shift(), Key::Space),
            (Modifiers::ALT, Key::J),
            (Modifiers::NONE, Key::F12),
            (
                Modifiers {
                    alt: true,
                    mac_cmd: true,
                    command: true,
                    ..Modifiers::NONE
                },
                Key::Backspace,
            ),
        ];
        for (mods, key) in cases {
            let Capture::Took(hk) = decide_capture(mods, key) else {
                panic!("{key:?} must be taken");
            };
            let shown = crate::hotkey::format_native(&hk);
            assert_eq!(
                crate::hotkey::parse(&shown).expect("the recorder's own output must parse"),
                hk,
                "round trip failed for {shown:?}"
            );
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
