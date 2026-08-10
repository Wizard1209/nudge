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
// Recorder decision logic — pure, no egui frame, fully unit-testable.
// =============================================================================

/// How long one combination must be held **unchanged** before the recorder
/// takes it.
///
/// The clock restarts on every change, so this is not a budget for assembling
/// the chord — it is how long you keep still once you have. Fast enough that
/// a deliberate press does not feel stuck, slow enough that passing through
/// `⌘` on the way to `⌥⌘J` does not record `⌘J`.
pub const HOLD_TO_COMMIT_SECS: f64 = 0.8;

/// What the shell should do with the recorder row this frame.
#[derive(Debug, Clone, PartialEq)]
pub enum RecorderStep {
    /// Nothing bindable is down. `modifiers` is what is held so far, so the
    /// row can show `⌥⌘…` while the user is still assembling.
    Waiting { modifiers: u8 },
    /// A registrable combination is held but not yet long enough. `progress`
    /// runs 0.0 → 1.0 and drives the bar the user is waiting on.
    Holding { hotkey: Hotkey, progress: f32 },
    /// Held for [`HOLD_TO_COMMIT_SECS`] — write it into the field and leave
    /// recording mode.
    Commit(Hotkey),
    /// Bare Escape — restore the previous label and leave recording mode.
    /// With any modifier held Escape is an ordinary key, so `⌃⎋` records.
    Cancel,
    /// Something is down that cannot become a hotkey. The string is shown to
    /// the user as-is and recording continues.
    Rejected(&'static str),
}

/// Tracks how long the current combination has been held. One lives in the
/// shell across frames; [`Self::step`] is the whole policy and takes the
/// clock as an argument so it can be tested without a `Context` — and so it
/// works on wasm, where `std::time::Instant` is not available.
#[derive(Debug, Clone, Default)]
pub struct HoldTracker {
    /// The combination being held and the timestamp it was first seen at.
    /// Cleared whenever the held combination changes or is released.
    held: Option<(Hotkey, f64)>,
}

impl HoldTracker {
    /// Advance one frame. `keys` is egui's set of non-modifier keys currently
    /// down; `now` is any monotonic clock in seconds (the shell passes
    /// `ctx.input(|i| i.time)`).
    ///
    /// Two keys at once is refused rather than resolved. egui reports
    /// `keys_down` as a `HashSet`, so "the first one" is whichever the hash
    /// order happened to put first — picking one would make the recorder
    /// non-deterministic for exactly the input where the user cannot tell
    /// which they got.
    pub fn step(
        &mut self,
        modifiers: eframe::egui::Modifiers,
        keys: &[eframe::egui::Key],
        now: f64,
    ) -> RecorderStep {
        use eframe::egui::Key;

        let held_modifiers = modifier_bits(modifiers);

        if keys.len() > 1 {
            self.held = None;
            return RecorderStep::Rejected("One key at a time, plus modifiers");
        }

        let Some(key) = keys.first().copied() else {
            self.held = None;
            return RecorderStep::Waiting {
                modifiers: held_modifiers,
            };
        };

        if key == Key::Escape && held_modifiers == 0 {
            self.held = None;
            return RecorderStep::Cancel;
        }

        let Some(hotkey) = hotkey::hotkey_from_egui(modifiers, key) else {
            self.held = None;
            return RecorderStep::Rejected("That key cannot be bound — try another");
        };

        // Any change to the combination restarts the clock, which is what
        // lets the user add a modifier mid-hold without recording the
        // partial chord.
        let since = match &self.held {
            Some((held, since)) if *held == hotkey => *since,
            _ => {
                self.held = Some((hotkey.clone(), now));
                now
            }
        };

        let elapsed = now - since;
        if elapsed >= HOLD_TO_COMMIT_SECS {
            self.held = None;
            RecorderStep::Commit(hotkey)
        } else {
            RecorderStep::Holding {
                hotkey,
                progress: (elapsed / HOLD_TO_COMMIT_SECS).clamp(0.0, 1.0) as f32,
            }
        }
    }
}

/// egui's modifier flags as our bit mask. Mirrors the modifier half of
/// [`hotkey::hotkey_from_egui`], which cannot be reused here because that one
/// needs a key to return anything at all — and the whole point of
/// [`RecorderStep::Waiting`] is showing the modifiers *before* there is one.
fn modifier_bits(m: eframe::egui::Modifiers) -> u8 {
    let mut bits = 0;
    if m.ctrl {
        bits |= hotkey::MOD_CTRL;
    }
    if m.alt {
        bits |= hotkey::MOD_ALT;
    }
    if m.shift {
        bits |= hotkey::MOD_SHIFT;
    }
    if m.mac_cmd {
        bits |= hotkey::MOD_WIN;
    }
    bits
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
    /// the row shows what is currently held plus a hold-progress bar, and
    /// per-frame input polling drives [`HoldTracker::step`].
    recording_hotkey: bool,
    /// Snapshot of the hotkey label taken when recording started — used by
    /// the cancel branch to restore the pre-recording value. `None` when
    /// we aren't recording.
    hotkey_pre_record: Option<String>,
    /// Hint shown below the row when the last thing held was not bindable.
    /// Cleared on entering / leaving recording mode.
    recording_hint: Option<&'static str>,
    /// How long the current combination has been held. Only meaningful while
    /// `recording_hotkey`; reset on entering and leaving capture mode.
    hold: HoldTracker,
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
            quit_requested: false,
            recording_hotkey: false,
            hotkey_pre_record: None,
            recording_hint: None,
            hold: HoldTracker::default(),
        }
    }

    /// Enter capture mode. Snapshot the current label so a Cancel/Esc can
    /// restore it; clear any stale hint from a previous session.
    fn start_recording(&mut self) {
        self.hotkey_pre_record = Some(self.form.hotkey.clone());
        self.recording_hotkey = true;
        self.recording_hint = None;
        self.hold = HoldTracker::default();
    }

    /// Leave capture mode and restore the pre-recording label. Used for both
    /// the explicit Cancel button and the bare-Esc keystroke.
    fn cancel_recording(&mut self) {
        if let Some(prev) = self.hotkey_pre_record.take() {
            self.form.hotkey = prev;
        }
        self.stop_recording();
    }

    /// Leave capture mode keeping whatever is in the field. The pre-record
    /// snapshot is dropped, so a later Cancel on the *window* cannot resurrect
    /// it.
    fn stop_recording(&mut self) {
        self.recording_hotkey = false;
        self.hotkey_pre_record = None;
        self.recording_hint = None;
        self.hold = HoldTracker::default();
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

        // Row 1: hotkey. Two modes — the editable TextEdit + "Record" button
        // when idle, and a live read-out of what is held + a hold-progress
        // bar while recording. Either way the row's product is the *text* in
        // `form.hotkey`; Save is what writes it to disk.
        //
        // The recorder polls before it draws, so the bar the user sees is
        // this frame's state rather than the previous one's — at 60 Hz over a
        // 0.8 s hold a frame of lag is visible as the bar arriving late and
        // finishing early.
        let step = if self.recording_hotkey {
            let (mods, keys, now) = ctx.input(|i| {
                let keys: Vec<egui::Key> = i.keys_down.iter().copied().collect();
                (i.modifiers, keys, i.time)
            });
            let step = self.hold.step(mods, &keys, now);
            match &step {
                RecorderStep::Cancel => self.cancel_recording(),
                RecorderStep::Commit(hk) => {
                    self.form.hotkey = hotkey::format_native(hk);
                    self.stop_recording();
                }
                RecorderStep::Rejected(hint) => self.recording_hint = Some(hint),
                RecorderStep::Waiting { .. } | RecorderStep::Holding { .. } => {
                    self.recording_hint = None
                }
            }
            // Keep repainting while recording so the next input frame arrives
            // promptly (eframe is otherwise lazy when no widget asks for it),
            // and so the progress bar animates rather than jumping on the
            // next unrelated event.
            ctx.request_repaint();
            Some(step)
        } else {
            None
        };

        ui.horizontal(|ui| {
            ui.label("Global hotkey:");
            match &step {
                // `step` is None exactly when we are not recording; the
                // matched arms below cover the recording row.
                None => {
                    ui.add(
                        egui::TextEdit::singleline(&mut self.form.hotkey)
                            .id(egui::Id::new("settings_hotkey"))
                            .desired_width(220.0)
                            .hint_text(hotkey::format_native(&hotkey::default_hotkey())),
                    );
                    if ui.button("Record").clicked() {
                        self.start_recording();
                    }
                }
                Some(step) => {
                    let (label, progress) = match step {
                        RecorderStep::Holding { hotkey, progress } => {
                            (hotkey::format_native(hotkey), *progress)
                        }
                        RecorderStep::Waiting { modifiers } if *modifiers != 0 => (
                            format!("{}…", hotkey::format_modifiers_native(*modifiers)),
                            0.0,
                        ),
                        _ => ("…".to_string(), 0.0),
                    };
                    // `desired_width`, not `add_sized`: a ProgressBar sizes
                    // itself to its text and ignores the outer allocation,
                    // which collapses it to a blob around the label. The
                    // width matches the TextEdit it replaces so the Record /
                    // Cancel button does not move when recording starts.
                    ui.add(
                        egui::ProgressBar::new(progress)
                            .desired_width(220.0)
                            .text(egui::RichText::new(label).strong()),
                    );
                    if ui.button("Cancel").clicked() {
                        self.cancel_recording();
                    }
                }
            }
        });
        // The instruction only exists while recording: outside it there is
        // nothing to hold and nothing Esc would cancel.
        if self.recording_hotkey {
            ui.label(
                egui::RichText::new("Hold the combination until the bar fills. Esc cancels.")
                    .small()
                    .weak(),
            );
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

    // ---- HoldTracker (recorder policy) --------------------------------------
    //
    // The clock is an argument, so these drive it directly: a frame is a call
    // with a timestamp, and holding for a second is two calls 1.0 apart.

    use eframe::egui::{Key, Modifiers};

    /// Ctrl+Shift, the chord most of these tests hold.
    fn ctrl_shift() -> Modifiers {
        Modifiers {
            ctrl: true,
            shift: true,
            ..Modifiers::NONE
        }
    }

    #[test]
    fn nothing_held_reports_the_modifiers_so_far() {
        let mut hold = HoldTracker::default();
        assert_eq!(
            hold.step(Modifiers::NONE, &[], 0.0),
            RecorderStep::Waiting { modifiers: 0 }
        );
        // Modifiers alone are not a hotkey, but the row shows them — that
        // read-out is the only feedback during the half-second it takes to
        // assemble a chord.
        assert_eq!(
            hold.step(ctrl_shift(), &[], 0.1),
            RecorderStep::Waiting {
                modifiers: crate::hotkey::MOD_CTRL | crate::hotkey::MOD_SHIFT
            }
        );
    }

    #[test]
    fn a_combination_commits_only_after_the_hold_elapses() {
        let mut hold = HoldTracker::default();
        assert!(matches!(
            hold.step(ctrl_shift(), &[Key::A], 10.0),
            RecorderStep::Holding { progress, .. } if progress == 0.0
        ));
        assert!(matches!(
            hold.step(ctrl_shift(), &[Key::A], 10.0 + HOLD_TO_COMMIT_SECS / 2.0),
            RecorderStep::Holding { progress, .. } if (progress - 0.5).abs() < 1e-6
        ));
        match hold.step(ctrl_shift(), &[Key::A], 10.0 + HOLD_TO_COMMIT_SECS) {
            RecorderStep::Commit(hk) => assert_eq!(crate::hotkey::format(&hk), "Ctrl+Shift+A"),
            other => panic!("expected Commit, got {other:?}"),
        }
    }

    #[test]
    fn adding_a_modifier_mid_hold_restarts_the_clock() {
        // The whole reason for hold-to-commit: reaching ⌃⇧A one key at a time
        // passes through ⌃A, and that intermediate must not be recorded.
        let mut hold = HoldTracker::default();
        hold.step(Modifiers::CTRL, &[Key::A], 0.0);
        let almost = HOLD_TO_COMMIT_SECS - 0.05;
        assert!(matches!(
            hold.step(Modifiers::CTRL, &[Key::A], almost),
            RecorderStep::Holding { .. }
        ));
        // Shift arrives just before Ctrl+A would have committed.
        assert!(
            matches!(
                hold.step(ctrl_shift(), &[Key::A], almost),
                RecorderStep::Holding { progress, .. } if progress == 0.0
            ),
            "a changed combination must start from zero, not inherit the old timer"
        );
        // And the old combination's elapsed time must not carry over.
        assert!(matches!(
            hold.step(ctrl_shift(), &[Key::A], almost + 0.05),
            RecorderStep::Holding { .. }
        ));
    }

    #[test]
    fn releasing_before_the_hold_elapses_records_nothing() {
        let mut hold = HoldTracker::default();
        hold.step(ctrl_shift(), &[Key::A], 0.0);
        assert_eq!(
            hold.step(Modifiers::NONE, &[], 0.3),
            RecorderStep::Waiting { modifiers: 0 }
        );
        // Pressing the same combination again starts over rather than
        // resuming — otherwise two taps would add up to a commit the user
        // never held.
        assert!(matches!(
            hold.step(ctrl_shift(), &[Key::A], 0.4),
            RecorderStep::Holding { progress, .. } if progress == 0.0
        ));
    }

    #[test]
    fn bare_escape_cancels_but_escape_with_a_modifier_is_bindable() {
        let mut hold = HoldTracker::default();
        assert_eq!(
            hold.step(Modifiers::NONE, &[Key::Escape], 0.0),
            RecorderStep::Cancel
        );
        // Cancel is immediate: a hold would make backing out of a recorder
        // that is already ignoring your keys take even longer.
        let mut hold = HoldTracker::default();
        hold.step(Modifiers::CTRL, &[Key::Escape], 0.0);
        match hold.step(Modifiers::CTRL, &[Key::Escape], HOLD_TO_COMMIT_SECS) {
            RecorderStep::Commit(hk) => assert_eq!(crate::hotkey::format(&hk), "Ctrl+Escape"),
            other => panic!("expected Commit(Ctrl+Escape), got {other:?}"),
        }
    }

    #[test]
    fn a_key_that_cannot_be_bound_is_refused_and_never_commits() {
        // Home is outside the allowlist the backends can register.
        let mut hold = HoldTracker::default();
        assert!(matches!(
            hold.step(Modifiers::CTRL, &[Key::Home], 0.0),
            RecorderStep::Rejected(_)
        ));
        assert!(
            matches!(
                hold.step(Modifiers::CTRL, &[Key::Home], HOLD_TO_COMMIT_SECS * 3.0),
                RecorderStep::Rejected(_)
            ),
            "holding an unbindable key longer must not eventually accept it"
        );
    }

    #[test]
    fn two_keys_at_once_are_refused_rather_than_arbitrated() {
        // egui hands us `keys_down` as a HashSet, so "the first key" is hash
        // order. Choosing one would record a combination the user did not
        // see and cannot predict.
        let mut hold = HoldTracker::default();
        assert!(matches!(
            hold.step(Modifiers::CTRL, &[Key::A, Key::B], 0.0),
            RecorderStep::Rejected(_)
        ));
    }

    #[test]
    fn a_second_key_landing_mid_hold_cancels_the_commit() {
        let mut hold = HoldTracker::default();
        hold.step(Modifiers::CTRL, &[Key::A], 0.0);
        assert!(matches!(
            hold.step(
                Modifiers::CTRL,
                &[Key::A, Key::B],
                HOLD_TO_COMMIT_SECS - 0.05
            ),
            RecorderStep::Rejected(_)
        ));
        // Back to one key: the clock must have been dropped, not paused.
        assert!(matches!(
            hold.step(Modifiers::CTRL, &[Key::A], HOLD_TO_COMMIT_SECS),
            RecorderStep::Holding { progress, .. } if progress == 0.0
        ));
    }

    #[test]
    fn what_the_recorder_commits_is_what_the_field_can_parse_back() {
        // The shell writes `format_native(committed)` into the field and Save
        // hands that string to the config. If the two disagree, the recorder
        // produces a hotkey the next launch cannot read.
        for (mods, key) in [
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
        ] {
            let mut hold = HoldTracker::default();
            hold.step(mods, &[key], 0.0);
            let RecorderStep::Commit(hk) = hold.step(mods, &[key], HOLD_TO_COMMIT_SECS) else {
                panic!("{key:?} must commit after the hold");
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
