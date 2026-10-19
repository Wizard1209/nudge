//! Settings window — opened from the tray menu as a separate process.
//!
//! Two layers, deliberately split:
//!
//! - [`SettingsForm`] is the **pure state machine**: it holds the field
//!   strings + bools, parses the numbers, tracks dirty state, and emits a
//!   `Config` on Save. No egui, no I/O. This is what's unit-tested.
//!
//! - [`SettingsApp`] is the eframe shell that renders rows, runs the sprite
//!   PNG check and `persist` on Save, and routes the autostart checkbox
//!   through [`crate::autostart::apply_autostart`] so the OS-confirms-first
//!   rule holds. The shell injects the persistence closure, the PNG check
//!   and the provider trait object so the same code drives the native
//!   (file + registry) and the WASM (localStorage + fake provider) builds.
//!
//! The render layer renders **rows by hand** rather than going through a
//! `Setting` trait — a handful of heterogeneous widgets don't justify the
//! abstraction yet, and inline rendering keeps the spec → screen mapping
//! direct.

use crate::autostart::{AutostartError, AutostartProvider, apply_autostart};
use crate::config::{Config, ConfigError, SpriteConfig};
use crate::hotkey::{self, Hotkey};
use crate::recorder::{self, EdgeSource, Effect, Input, Recorder};
use std::path::Path;

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
#[derive(Debug, Clone, PartialEq)]
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
    /// The §10 sprite row. The numbers are field text, parsed strictly on
    /// Save — whole numbers of points, like the interval is a number of
    /// minutes — and validated whether or not the sprite is on, so a typo
    /// cannot sit in the file waiting for the checkbox.
    pub sprite_enabled: bool,
    pub sprite_path: String,
    pub sprite_x_text: String,
    pub sprite_y_text: String,
    pub card_width_text: String,
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
            sprite_enabled: cfg.sprite.enabled,
            sprite_path: cfg.sprite.path.clone(),
            sprite_x_text: cfg.sprite.x.to_string(),
            sprite_y_text: cfg.sprite.y.to_string(),
            card_width_text: cfg.sprite.card_width.to_string(),
            original: cfg,
        }
    }

    /// Parse `interval_text` into a finite-positive f64, with the same
    /// forgiveness as [`Config::resolved_interval_minutes`].
    pub fn parsed_interval(&self) -> Result<f64, IntervalParseError> {
        parse_interval_minutes(&self.interval_text)
    }

    /// Whether any field differs from the file — what greys `Save` and gates
    /// [`SettingsApp::save`] (§9).
    pub fn is_dirty(&self) -> bool {
        self.hotkey != self.original.hotkey
            || self.interval_text != format_interval(self.original.default_interval_minutes)
            || self.autostart != self.original.autostart
            || self.sprite_enabled != self.original.sprite.enabled
            || self.sprite_path != self.original.sprite.path
            || self.sprite_x_text != self.original.sprite.x.to_string()
            || self.sprite_y_text != self.original.sprite.y.to_string()
            || self.card_width_text != self.original.sprite.card_width.to_string()
    }

    /// The hotkey as last saved — what every cancelling path in the recorder
    /// puts back.
    ///
    /// The *saved* value, deliberately, not a copy of the field taken when the
    /// recorder armed: cancelling has one meaning, and a typed-but-unsaved edit
    /// goes with it. That is what lets the recorder hold no state at all.
    pub fn saved_hotkey(&self) -> &str {
        &self.original.hotkey
    }

    /// The typed hotkey, parsed. The field accepts every spelling
    /// [`hotkey::parse`] does — `⌥⌘J`, `Alt+Cmd+J`, `alt+cmd+j` — and every
    /// one of them means the same combination.
    pub fn parsed_hotkey(&self) -> Result<Hotkey, hotkey::ParseError> {
        hotkey::parse(&self.hotkey)
    }

    /// Build a `Config` from the current form, validating every typed
    /// field. The `autostart` field is taken from the form's bool (which the
    /// caller keeps in sync with the OS via `apply_autostart`). On a
    /// successful save, the caller should call `mark_clean` so subsequent
    /// dirty checks don't keep tripping.
    pub fn to_config(&self) -> Result<Config, SettingsValidationError> {
        // Every field is validated before any is written, so a bad hotkey
        // cannot leave a saved interval next to an unsaved combo. One
        // refusal per save, the first in row order.
        let hotkey = self
            .parsed_hotkey()
            .map_err(SettingsValidationError::Hotkey)?;
        let interval = self
            .parsed_interval()
            .map_err(SettingsValidationError::Interval)?;
        let x = parse_points(&self.sprite_x_text)
            .ok_or_else(|| SettingsValidationError::SpriteX(self.sprite_x_text.trim().into()))?;
        let y = parse_points(&self.sprite_y_text)
            .ok_or_else(|| SettingsValidationError::SpriteY(self.sprite_y_text.trim().into()))?;
        let card_width = parse_points(&self.card_width_text)
            .and_then(|w| u32::try_from(w).ok().filter(|w| *w > 0))
            .ok_or_else(|| {
                SettingsValidationError::CardWidth(self.card_width_text.trim().into())
            })?;
        Ok(Config {
            // Canonical, not as-typed: the file gets one spelling per
            // combination, and the field is rehydrated with it on save so the
            // user sees what was actually stored.
            hotkey: hotkey::format_native(&hotkey),
            default_interval_minutes: interval,
            autostart: self.autostart,
            sprite: SpriteConfig {
                enabled: self.sprite_enabled,
                path: self.sprite_path.trim().to_string(),
                x,
                y,
                card_width,
            },
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
        self.sprite_enabled = saved.sprite.enabled;
        self.sprite_path = saved.sprite.path.clone();
        self.sprite_x_text = saved.sprite.x.to_string();
        self.sprite_y_text = saved.sprite.y.to_string();
        self.card_width_text = saved.sprite.card_width.to_string();
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
            sprite: self.original.sprite.clone(),
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
    /// The sprite row's numbers; each carries the trimmed input.
    SpriteX(String),
    SpriteY(String),
    CardWidth(String),
}

impl std::fmt::Display for SettingsValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingsValidationError::Interval(e) => write!(f, "{e}"),
            // The parse error says what is wrong with the string; this adds
            // what a correct one looks like, because the banner is the only
            // place the accepted vocabulary is stated.
            SettingsValidationError::Hotkey(e) => write!(f, "{e}. {}", hotkey::VOCABULARY),
            SettingsValidationError::SpriteX(_) => {
                f.write_str("Sprite X must be a whole number of points")
            }
            SettingsValidationError::SpriteY(_) => {
                f.write_str("Sprite Y must be a whole number of points")
            }
            SettingsValidationError::CardWidth(_) => {
                f.write_str("Card width must be a positive whole number of points")
            }
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

/// A whole number of points, optionally signed: `^-?\d+$` on the trimmed
/// text, nothing looser. Strict like the interval — the settings window is
/// where a typo is surfaced, not papered over.
fn parse_points(text: &str) -> Option<i32> {
    let trimmed = text.trim();
    let digits = trimmed.strip_prefix('-').unwrap_or(trimmed);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    trimmed.parse().ok()
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
// eframe shell — render & event-loop layer. Cross-platform so it compiles on
// native AND wasm32; the *callers* differ (native main spawns it as a
// dedicated process, wasm boots it via the URL query branch in lib.rs).
// =============================================================================

/// The hotkey row's recorder: the pure table, the row's own hint line, and
/// the key source that feeds it.
///
/// It owns no copy of the field and nothing it would have to remember at arm
/// time — every cancelling path reads [`SettingsForm::saved_hotkey`]. What it
/// does own is the arming of the source, because arming is what makes keys
/// stop reaching the window and the two must not be able to disagree: the
/// state changes and the source is told, in one place.
pub struct HotkeyRecorder {
    state: Recorder,
    /// The line under the row while armed. `None` when idle, where the row
    /// shows the standing vocabulary instead.
    hint: Option<String>,
    /// `None` on a platform with no key source — Windows and the browser
    /// build. The row then renders exactly the typed field it always had, with
    /// no Record button, because a button that cannot record is worse than no
    /// button.
    source: Option<Box<dyn EdgeSource>>,
    /// Last frame's focus, so the *transition* can be turned into one
    /// `FocusLost` input rather than an input per unfocused frame.
    was_focused: bool,
}

impl HotkeyRecorder {
    pub fn new(source: Option<Box<dyn EdgeSource>>) -> Self {
        Self {
            state: Recorder::Idle,
            hint: None,
            source,
            was_focused: true,
        }
    }

    /// Whether this platform can record at all.
    pub fn available(&self) -> bool {
        self.source.is_some()
    }

    pub fn is_armed(&self) -> bool {
        self.state == Recorder::Armed
    }

    /// The line to show under the row: the prompt, or the refusal that names
    /// the key that was pressed.
    pub fn hint(&self) -> Option<&str> {
        self.hint.as_deref()
    }

    /// Put one input through the table and keep the row's own state in step
    /// with the answer. The returned effect is the caller's — it is the only
    /// thing that touches the field.
    pub fn feed(&mut self, input: &Input) -> Effect {
        let (next, effect) = recorder::step(self.state, input);
        if next != self.state {
            self.state = next;
            if let Some(source) = self.source.as_mut() {
                source.set_armed(next == Recorder::Armed);
            }
            self.hint = (next == Recorder::Armed).then(|| recorder::PROMPT.to_string());
        }
        if let Effect::Hint(text) = &effect {
            self.hint = Some(text.clone());
        }
        effect
    }

    /// Everything that arrived since the last frame, in order: the focus
    /// transition first, then each press edge, each already judged by the
    /// source that produced it.
    pub fn pump(&mut self, focused: bool) -> Vec<Effect> {
        let mut effects = Vec::new();
        if self.was_focused && !focused {
            effects.push(self.feed(&Input::FocusLost));
        }
        self.was_focused = focused;

        let edges = match self.source.as_mut() {
            Some(source) => source.drain(),
            None => return effects,
        };
        for edge in edges {
            // Classified by the source, before the table sees it: whether a
            // key is bindable is decided outside the table and handed in
            // already answered.
            let Some(verdict) = self.source.as_ref().map(|s| s.classify(edge)) else {
                break;
            };
            effects.push(self.feed(&Input::Press(verdict)));
        }
        effects
    }
}

/// Persistence closure: native writes to disk, WASM writes to localStorage.
/// Boxed so the SettingsApp can hold one across the run loop without a
/// generic parameter on the App impl (eframe::App is dyn-friendly that way).
pub type PersistFn = Box<dyn FnMut(&Config) -> Result<(), ConfigError> + Send>;

/// The §10 PNG check Save runs on an enabled sprite: `Err` is the reason the
/// window shows. Native injects `sprite_png::check_sprite_file`; the browser
/// build, with no file access, injects a check that accepts everything.
pub type SpriteCheckFn = Box<dyn Fn(&Path) -> Result<(), String> + Send>;

/// Save, as the form sees it: validate, check the sprite file when the
/// sprite is on, write, rebaseline. `Ok` carries what reached the disk;
/// `Err` the one line the window shows instead, and nothing was written.
pub fn save_form(
    form: &mut SettingsForm,
    sprite_check: &SpriteCheckFn,
    persist: &mut PersistFn,
) -> Result<Config, String> {
    let cfg = form.to_config().map_err(|e| e.to_string())?;
    // §9: an enabled sprite must be a usable PNG; a disabled one saves as
    // typed, without touching the file at all.
    if cfg.sprite.enabled {
        sprite_check(Path::new(&cfg.sprite.path))?;
    }
    persist(&cfg).map_err(|e| format!("Save failed: {e}"))?;
    form.mark_clean(cfg.clone());
    Ok(cfg)
}

/// Put one effect into the form. The effect is the only thing that touches
/// the field, and of the four exactly two do.
///
/// Free-standing rather than a method on the shell, so the replay tier drives
/// the same code the window does instead of a copy of it that could drift.
/// Returns whether a combination was *captured*.
pub fn apply_effect(effect: &Effect, form: &mut SettingsForm) -> bool {
    match effect {
        // A hint is the row's own line under the field; it changes no value,
        // and the recorder is still armed behind it.
        Effect::Nothing | Effect::Hint(_) => false,
        Effect::Capture(hk) => {
            form.hotkey = hotkey::format_native(hk);
            true
        }
        Effect::Restore => {
            form.hotkey = form.saved_hotkey().to_string();
            false
        }
    }
}

/// Ask the platform who else holds this combination. `None` everywhere the
/// question cannot be answered — the browser build, and Windows, whose
/// arbitration is documented but unmeasured.
fn conflict_warning(hk: &Hotkey) -> Option<String> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        crate::platform::hotkey_conflict(hk)
    }
    #[cfg(target_arch = "wasm32")]
    {
        let _ = hk;
        None
    }
}

/// The OS file picker for the sprite row. `None` when the user cancels — the
/// field keeps what it had. Blocking, on the main thread, like the window
/// itself.
// TODO: Windows Browse… unverified on a desktop.
#[cfg(any(target_os = "windows", target_os = "macos"))]
fn pick_sprite_file() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("Choose a sprite")
        .add_filter("PNG image", &["png"])
        .pick_file()
        .map(|p| p.display().to_string())
}

/// The form's text fields, by egui id. §9 "Return anywhere in the form is
/// Save" is decided against this list: the focus is in the form when it is in
/// one of these or nowhere at all, and a focused button keeps its own Return,
/// which egui already makes a click.
const ID_HOTKEY: &str = "settings_hotkey";
const ID_INTERVAL: &str = "settings_interval";
const ID_SPRITE_PATH: &str = "settings_sprite_path";
const ID_SPRITE_X: &str = "settings_sprite_x";
const ID_SPRITE_Y: &str = "settings_sprite_y";
const ID_CARD_WIDTH: &str = "settings_card_width";
const FIELD_IDS: [&str; 6] = [
    ID_HOTKEY,
    ID_INTERVAL,
    ID_SPRITE_PATH,
    ID_SPRITE_X,
    ID_SPRITE_Y,
    ID_CARD_WIDTH,
];

/// The settings window as an eframe::App. Owns the form state, the provider,
/// and the persistence closure; renders three rows + Save/Cancel + an error
/// banner.
pub struct SettingsApp {
    form: SettingsForm,
    provider: Box<dyn AutostartProvider>,
    persist: PersistFn,
    sprite_check: SpriteCheckFn,
    /// The hotkey row's recorder — see [`HotkeyRecorder`].
    recorder: HotkeyRecorder,
    /// Surfaced to the user above the buttons. Cleared on the next interaction.
    banner: Option<String>,
    /// §9: a save that reached the file says so, beside the buttons, until
    /// the first edit after it.
    saved: bool,
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
        sprite_check: SpriteCheckFn,
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
        // The creator closure is the one place with a live `NSApplication` on
        // the main thread, which is what installing a key monitor needs — the
        // same precondition the status item has.
        #[cfg(not(target_arch = "wasm32"))]
        let source = crate::platform::new_edge_source(&cc.egui_ctx);
        #[cfg(target_arch = "wasm32")]
        let source = None;
        Self {
            form: SettingsForm::from_config(config),
            provider,
            persist,
            sprite_check,
            recorder: HotkeyRecorder::new(source),
            banner: None,
            saved: false,
            quit_requested: false,
        }
    }

    /// Apply everything the recorder produced this frame.
    fn apply_recorder_effects(&mut self, focused: bool) {
        for effect in self.recorder.pump(focused) {
            if apply_effect(&effect, &mut self.form) {
                // A capture answers whatever the banner was complaining about.
                self.banner = None;
            }
        }
    }

    /// Save path, for the button and for Return alike: [`save_form`], then
    /// the `Saved` note or the banner.
    fn save(&mut self) {
        // §9: Save is live only while the form differs from the file — the
        // one gate for the button and the key, so a greyed button and a
        // Return on a clean form mean the same nothing.
        if !self.form.is_dirty() {
            return;
        }
        match save_form(&mut self.form, &self.sprite_check, &mut self.persist) {
            Ok(saved) => {
                self.saved = true;
                // Who else holds it is a warning next to a successful save,
                // never a refusal: the OS accepts a taken combination and
                // registers it, it simply never fires. That is a different
                // thing from the *unregistrable* one §5 blocks the save on,
                // which never gets this far.
                self.banner = hotkey::parse(&saved.hotkey)
                    .ok()
                    .and_then(|hk| conflict_warning(&hk))
                    .map(|warning| format!("This combination is {warning}"));
            }
            Err(line) => self.banner = Some(line),
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

        // What the form held when the frame began: §9 takes the `Saved` note
        // away on the first edit after a save, and an edit is the form
        // differing at the end of the frame from this — whichever row, key,
        // click or capture made it differ.
        let before = self.form.clone();

        // Everything the keyboard did since the last frame, before anything is
        // drawn — so a press and the field it changes land in the same frame.
        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        self.apply_recorder_effects(focused);

        // Esc closes the window. While armed it never gets here: the key
        // source swallows what it reports, which is the same reason ⌘C does
        // not copy and Tab does not move the focus.
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.quit_requested = true;
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            self.quit_requested = true;
        }

        let ui = root_ui;
        ui.heading("Settings");
        ui.add_space(8.0);

        // Row 1: hotkey — typed in the §5 grammar, or pressed.
        //
        // Armed, the field is not shown at all: it holds the *saved* value
        // until a press replaces it, and a text box under a "press a
        // combination" prompt reads as somewhere to type.
        let armed = self.recorder.is_armed();
        ui.horizontal(|ui| {
            ui.label("Global hotkey:");
            if armed {
                ui.label(
                    egui::RichText::new(recorder::PROMPT)
                        .italics()
                        .color(egui::Color32::LIGHT_YELLOW),
                );
            } else {
                ui.add(
                    egui::TextEdit::singleline(&mut self.form.hotkey)
                        .id(egui::Id::new(ID_HOTKEY))
                        .desired_width(220.0)
                        .hint_text(hotkey::format_native(&hotkey::default_hotkey())),
                );
            }
            // No button at all where nothing can record — a Record button that
            // cannot record is worse than none.
            if self.recorder.available() {
                let label = if armed { "Cancel" } else { "Record" };
                if ui.button(label).clicked() {
                    let input = if armed { Input::Cancel } else { Input::Record };
                    let effect = self.recorder.feed(&input);
                    if effect == Effect::Restore {
                        self.form.hotkey = self.form.saved_hotkey().to_string();
                    }
                }
            }
        });
        // Under the row: while armed, what the recorder has to say — the
        // prompt, or a refusal naming the key. Otherwise what may be typed,
        // which is standing information rather than an error message: this is
        // how a hotkey is set.
        let under = self.recorder.hint().unwrap_or(hotkey::VOCABULARY);
        ui.label(egui::RichText::new(under).small().weak());
        ui.add_space(6.0);

        // Row 2: default interval
        ui.horizontal(|ui| {
            ui.label("Default interval (min):");
            ui.add(
                egui::TextEdit::singleline(&mut self.form.interval_text)
                    .id(egui::Id::new(ID_INTERVAL))
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
        ui.add_space(6.0);

        // Row 4: the §10 sprite — on/off, the PNG, where it sits and how wide
        // the card is under it. The file is checked on Save, not here.
        ui.checkbox(&mut self.form.sprite_enabled, "Sprite");
        ui.horizontal(|ui| {
            ui.label("PNG:");
            ui.add(
                egui::TextEdit::singleline(&mut self.form.sprite_path)
                    .id(egui::Id::new(ID_SPRITE_PATH))
                    .desired_width(300.0)
                    .hint_text("Path to a PNG with a transparent background"),
            );
            // A native file dialog takes the focus, which would end a
            // recording anyway; not offering it while armed keeps the two
            // from racing. The browser build has no dialog at all.
            #[cfg(any(target_os = "windows", target_os = "macos"))]
            if ui
                .add_enabled(!armed, egui::Button::new("Browse…"))
                .clicked()
                && let Some(picked) = pick_sprite_file()
            {
                self.form.sprite_path = picked;
            }
        });
        ui.horizontal(|ui| {
            ui.label("X (pt):");
            ui.add(
                egui::TextEdit::singleline(&mut self.form.sprite_x_text)
                    .id(egui::Id::new(ID_SPRITE_X))
                    .desired_width(60.0)
                    .hint_text("0"),
            );
            ui.label("Y (pt):");
            ui.add(
                egui::TextEdit::singleline(&mut self.form.sprite_y_text)
                    .id(egui::Id::new(ID_SPRITE_Y))
                    .desired_width(60.0)
                    .hint_text("0"),
            );
            ui.label("Card width (pt):");
            ui.add(
                egui::TextEdit::singleline(&mut self.form.card_width_text)
                    .id(egui::Id::new(ID_CARD_WIDTH))
                    .desired_width(60.0)
                    .hint_text(crate::config::DEFAULT_CARD_WIDTH.to_string()),
            );
        });
        ui.add_space(12.0);

        // Error / status banner
        if let Some(msg) = &self.banner {
            ui.colored_label(egui::Color32::LIGHT_YELLOW, msg);
            ui.add_space(8.0);
        }

        if self.form != before {
            self.saved = false;
        }

        // §9: Return anywhere in the form is Save. "In the form" is the focus
        // in one of the fields or nowhere; a focused button or checkbox keeps
        // its own Return, which egui already makes a click. While the recorder
        // is armed the key is the recorder's — a press, not a save.
        let return_in_the_form = !armed
            && ctx.input(|i| i.key_pressed(egui::Key::Enter))
            && ctx
                .memory(|m| m.focused())
                .is_none_or(|id| FIELD_IDS.iter().any(|name| egui::Id::new(name) == id));

        let mut save_clicked = false;
        ui.horizontal(|ui| {
            // Greyed while nothing differs from the file (§9).
            save_clicked = ui
                .add_enabled(self.form.is_dirty(), egui::Button::new("Save"))
                .clicked();
            if ui.button("Cancel").clicked() {
                self.quit_requested = true;
            }
            if self.saved {
                ui.label(egui::RichText::new("Saved").weak());
            }
        });
        if save_clicked || return_in_the_form {
            self.save();
        }

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
            ..Config::default()
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
        // accepted forms was typed.
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

    fn sprite_on() -> SpriteConfig {
        SpriteConfig {
            enabled: true,
            path: "/a/b.png".to_string(),
            x: -41,
            y: -277,
            card_width: 679,
        }
    }

    #[test]
    fn to_config_carries_the_sprite_block() {
        // The form has no sprite row yet; Save must still not delete the
        // block a hand-edited file carries.
        let f = SettingsForm::from_config(Config {
            sprite: sprite_on(),
            ..Config::default()
        });
        assert!(!f.is_dirty());
        assert_eq!(f.to_config().unwrap().sprite, sprite_on());
    }

    #[test]
    fn toggle_autostart_keeps_the_sprite_block() {
        let provider = FakeProvider::new(false);
        let mut f = SettingsForm::from_config(Config {
            sprite: sprite_on(),
            ..Config::default()
        });
        let saved = std::cell::RefCell::new(None);

        f.toggle_autostart(&provider, true, |c| {
            *saved.borrow_mut() = Some(c.clone());
            Ok(())
        })
        .unwrap();

        assert_eq!(
            saved.borrow().as_ref().map(|c| c.sprite.clone()),
            Some(sprite_on()),
            "the persisted config carries the sprite block"
        );
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

    // === the hotkey row's recorder ==========================================
    //
    // Buttons and focus only. Not one of these feeds a key: what a keypress
    // does is asserted against streams a keyboard actually produced, in
    // `tests/hotkey_replay.rs`, and a hand-written key event here would be the
    // suite that passes while the recorder is dead.

    #[test]
    fn a_platform_with_no_key_source_shows_no_record_button() {
        // Windows and the browser build. The row is the typed field it always
        // was — a Record button that cannot record is worse than none.
        let row = HotkeyRecorder::new(None);
        assert!(!row.available());
        assert!(!row.is_armed());
        assert_eq!(
            row.hint(),
            None,
            "the vocabulary line stands where no hint is"
        );
    }

    #[test]
    fn the_record_button_arms_and_puts_the_prompt_under_the_row() {
        let mut row = HotkeyRecorder::new(None);
        assert_eq!(row.feed(&Input::Record), Effect::Nothing);
        assert!(row.is_armed());
        assert_eq!(row.hint(), Some(recorder::PROMPT));
    }

    #[test]
    fn cancelling_restores_the_saved_hotkey_over_a_typed_edit() {
        // The field may hold something the user typed and never saved. Cancel
        // has one meaning, and that edit goes with it — which is exactly why
        // the recorder remembers nothing when it arms.
        let mut form = SettingsForm::from_config(cfg_with("Ctrl+Shift+Space", 10.0, false));
        form.hotkey = "Alt+Q".to_string();
        let mut row = HotkeyRecorder::new(None);
        row.feed(&Input::Record);

        assert_eq!(row.feed(&Input::Cancel), Effect::Restore);
        assert!(!row.is_armed());
        form.hotkey = form.saved_hotkey().to_string();
        assert_eq!(form.hotkey, native("Ctrl+Shift+Space"));
        assert_eq!(row.hint(), None, "the hint goes when the recorder does");
    }

    #[test]
    fn losing_the_focus_ends_the_recording() {
        // *Which* shortcut took the focus is not observable — macOS eats ⌘Tab
        // and ⌘Space before any app sees them — so the transition is the whole
        // signal, and it fires once rather than every unfocused frame.
        let mut row = HotkeyRecorder::new(None);
        row.feed(&Input::Record);
        assert_eq!(row.pump(false), vec![Effect::Restore]);
        assert!(!row.is_armed());
        assert_eq!(
            row.pump(false),
            vec![],
            "still unfocused is not a new event"
        );
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

    // === the sprite row (port of electron/tests/unit/settingsForm.test.ts) ===

    fn sprite_cfg() -> Config {
        Config {
            sprite: SpriteConfig {
                enabled: true,
                path: "C:\\art\\s.png".to_string(),
                x: -41,
                y: -277,
                card_width: 679,
            },
            ..Config::default()
        }
    }

    #[test]
    fn sprite_fields_seed_from_config() {
        let f = SettingsForm::from_config(sprite_cfg());
        assert!(f.sprite_enabled);
        assert_eq!(f.sprite_path, "C:\\art\\s.png");
        assert_eq!(
            [&f.sprite_x_text, &f.sprite_y_text, &f.card_width_text],
            ["-41", "-277", "679"]
        );
        assert!(!f.is_dirty());
    }

    #[test]
    fn to_config_trims_sprite_fields() {
        let mut f = SettingsForm::from_config(Config::default());
        f.sprite_enabled = true;
        f.sprite_path = "  C:\\art\\s.png ".to_string();
        f.sprite_x_text = " -41".to_string();
        f.sprite_y_text = "-277 ".to_string();
        f.card_width_text = "679".to_string();
        assert_eq!(f.to_config().unwrap().sprite, sprite_cfg().sprite);
    }

    #[test]
    fn editing_each_sprite_field_marks_dirty() {
        let edits: [fn(&mut SettingsForm); 5] = [
            |f| f.sprite_enabled = true,
            |f| f.sprite_path = "/x.png".to_string(),
            |f| f.sprite_x_text = "5".to_string(),
            |f| f.sprite_y_text = "5".to_string(),
            |f| f.card_width_text = "500".to_string(),
        ];
        for (i, edit) in edits.iter().enumerate() {
            let mut f = SettingsForm::from_config(Config::default());
            edit(&mut f);
            assert!(f.is_dirty(), "sprite edit {i} -> dirty");
        }
    }

    #[test]
    fn sprite_x_rejects_non_integers() {
        for bad in ["", "1.5", "abc", "1e3x", " 7 x "] {
            let mut f = SettingsForm::from_config(Config::default());
            f.sprite_x_text = bad.to_string();
            assert_eq!(
                f.to_config().unwrap_err(),
                SettingsValidationError::SpriteX(bad.trim().to_string())
            );
        }
        assert_eq!(
            SettingsValidationError::SpriteX("abc".to_string()).to_string(),
            "Sprite X must be a whole number of points"
        );
    }

    #[test]
    fn sprite_y_rejects_non_integers() {
        for bad in ["", "1.5", "abc", "1e3x"] {
            let mut f = SettingsForm::from_config(Config::default());
            f.sprite_y_text = bad.to_string();
            assert_eq!(
                f.to_config().unwrap_err(),
                SettingsValidationError::SpriteY(bad.trim().to_string())
            );
        }
        assert_eq!(
            SettingsValidationError::SpriteY("abc".to_string()).to_string(),
            "Sprite Y must be a whole number of points"
        );
    }

    #[test]
    fn card_width_rejects_a_non_positive_or_non_integer() {
        for bad in ["", "0", "-480", "480.5", "wide"] {
            let mut f = SettingsForm::from_config(Config::default());
            f.card_width_text = bad.to_string();
            assert_eq!(
                f.to_config().unwrap_err(),
                SettingsValidationError::CardWidth(bad.trim().to_string())
            );
        }
        assert_eq!(
            SettingsValidationError::CardWidth("0".to_string()).to_string(),
            "Card width must be a positive whole number of points"
        );
    }

    #[test]
    fn number_fields_are_validated_while_the_sprite_is_off() {
        let mut f = SettingsForm::from_config(Config::default());
        assert!(!f.sprite_enabled);
        f.sprite_x_text = "x".to_string();
        assert!(matches!(
            f.to_config(),
            Err(SettingsValidationError::SpriteX(_))
        ));
    }

    #[test]
    fn validation_order_is_hotkey_interval_x_y_width() {
        // §9: one field is reported per refused save, the first in row order.
        let mut f = SettingsForm::from_config(Config::default());
        f.hotkey = "Ctrl+Shift".to_string();
        f.interval_text = "x".to_string();
        f.sprite_x_text = "x".to_string();
        f.sprite_y_text = "x".to_string();
        f.card_width_text = "x".to_string();
        assert!(matches!(
            f.to_config(),
            Err(SettingsValidationError::Hotkey(_))
        ));
        f.hotkey = native("Alt+J");
        assert!(matches!(
            f.to_config(),
            Err(SettingsValidationError::Interval(_))
        ));
        f.interval_text = "10".to_string();
        assert!(matches!(
            f.to_config(),
            Err(SettingsValidationError::SpriteX(_))
        ));
        f.sprite_x_text = "0".to_string();
        assert!(matches!(
            f.to_config(),
            Err(SettingsValidationError::SpriteY(_))
        ));
        f.sprite_y_text = "0".to_string();
        assert!(matches!(
            f.to_config(),
            Err(SettingsValidationError::CardWidth(_))
        ));
        f.card_width_text = "480".to_string();
        assert!(f.to_config().is_ok());
    }

    #[test]
    fn mark_clean_rebaselines_sprite_fields() {
        let mut f = SettingsForm::from_config(Config::default());
        f.sprite_enabled = true;
        f.sprite_path = " /x.png ".to_string();
        f.sprite_x_text = " 7 ".to_string();
        let saved = f.to_config().unwrap();
        f.mark_clean(saved);
        assert!(!f.is_dirty());
        assert_eq!([f.sprite_path, f.sprite_x_text], ["/x.png", "7"]);
    }

    // === Save: the PNG check sits between validation and the write =========

    fn recording_persist() -> (PersistFn, std::sync::Arc<std::sync::Mutex<Vec<Config>>>) {
        let persisted = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let writer = std::sync::Arc::clone(&persisted);
        let persist: PersistFn = Box::new(move |c: &Config| {
            writer.lock().unwrap().push(c.clone());
            Ok(())
        });
        (persist, persisted)
    }

    #[test]
    fn an_enabled_sprite_that_fails_the_check_is_not_persisted() {
        let reason = crate::sprite_png::SpriteError::Opaque.to_string();
        let refused = reason.clone();
        let check: SpriteCheckFn = Box::new(move |_: &Path| Err(refused.clone()));
        let (mut persist, persisted) = recording_persist();
        let mut f = SettingsForm::from_config(sprite_cfg());

        let outcome = save_form(&mut f, &check, &mut persist);

        assert_eq!(outcome, Err(reason), "the message line is the reason");
        assert!(persisted.lock().unwrap().is_empty(), "nothing is written");
    }

    #[test]
    fn a_disabled_sprite_saves_without_a_file_check() {
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = std::sync::Arc::clone(&calls);
        let check: SpriteCheckFn = Box::new(move |_: &Path| {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Err("never asked".to_string())
        });
        let (mut persist, persisted) = recording_persist();
        let mut f = SettingsForm::from_config(Config::default());
        f.sprite_path = "/nowhere.png".to_string();

        let outcome = save_form(&mut f, &check, &mut persist);

        assert!(outcome.is_ok(), "{outcome:?}");
        assert_eq!(persisted.lock().unwrap().len(), 1, "written once");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 0);
        assert!(!f.is_dirty());
    }

    // === §9 Saving: Save is live only while the form differs from the file ==

    /// §9 Saving: "a form that reads as unedited is one whose content is on
    /// disk" — what the form compares against after a save is what the
    /// closure was handed, not what was typed.
    #[test]
    fn a_save_that_reached_the_file_leaves_nothing_to_save() {
        let check: SpriteCheckFn = Box::new(|_: &Path| Ok(()));
        let (mut persist, persisted) = recording_persist();
        let mut f = SettingsForm::from_config(cfg_with("Alt+J", 7.0, false));
        f.interval_text = " 12 ".to_string();
        assert!(f.is_dirty());

        let written = save_form(&mut f, &check, &mut persist).expect("a plain save");

        assert_eq!(
            persisted.lock().unwrap().as_slice(),
            std::slice::from_ref(&written)
        );
        assert!(!f.is_dirty(), "the form now reads as unedited");
        assert_eq!(
            f.to_config().unwrap(),
            written,
            "and its content is on disk"
        );
    }

    /// §9 Saving: "`Save` is live only while the form differs from the file"
    /// — a refused save changes nothing on disk, so the form still differs
    /// and Save stays live.
    #[test]
    fn a_refused_save_leaves_the_form_dirty() {
        let check: SpriteCheckFn = Box::new(|_: &Path| Ok(()));
        let (mut persist, persisted) = recording_persist();
        let mut f = SettingsForm::from_config(cfg_with("Alt+J", 7.0, false));
        f.interval_text = "x".to_string();

        let outcome = save_form(&mut f, &check, &mut persist);

        assert!(outcome.is_err());
        assert!(persisted.lock().unwrap().is_empty());
        assert!(
            f.is_dirty(),
            "nothing reached the file, so the form still differs"
        );
    }
}
