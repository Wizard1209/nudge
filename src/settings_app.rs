use crate::autostart::{AutostartError, AutostartProvider, apply_autostart};
use crate::config::{self, Config, ConfigError, IntervalError, SpriteConfig};
use crate::hotkey::{self, Hotkey};
use crate::recorder::{self, EdgeSource, Effect, Input, Recorder, Verdict};
use eframe::egui;
use std::path::Path;

pub const SETTINGS_ARG: &str = "--settings";

pub const TRAY_THREAD_ARG: &str = "--tray-thread";

pub fn parse_tray_thread_arg<I, S>(args: I) -> Option<u32>
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        if a.as_ref() == TRAY_THREAD_ARG {
            return args.next()?.as_ref().parse().ok();
        }
    }
    None
}

pub fn parse_settings_arg<I, S>(args: I) -> bool
where
    I: IntoIterator<Item = S>,
    S: AsRef<str>,
{
    args.into_iter().any(|a| a.as_ref() == SETTINGS_ARG)
}

#[derive(Debug, Clone, PartialEq)]
pub struct SettingsForm {
    pub hotkey: String,
    pub interval_text: String,
    pub autostart: bool,
    pub sprite_enabled: bool,
    pub sprite_path: String,
    pub sprite_x_text: String,
    pub sprite_y_text: String,
    pub card_width_text: String,
    original: Config,
}

impl SettingsForm {
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

    pub fn parsed_interval(&self) -> Result<f64, IntervalError> {
        parse_interval_minutes(&self.interval_text)
    }

    pub fn is_dirty(&self) -> bool {
        *self != Self::from_config(self.original.clone())
    }

    pub fn saved_hotkey(&self) -> &str {
        &self.original.hotkey
    }

    pub fn parsed_hotkey(&self) -> Result<Hotkey, hotkey::ParseError> {
        hotkey::parse(&self.hotkey)
    }

    pub fn to_config(&self) -> Result<Config, SettingsValidationError> {
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

    pub fn mark_clean(&mut self, saved: Config) {
        *self = Self::from_config(saved);
    }

    pub fn toggle_autostart<F>(
        &mut self,
        provider: &dyn AutostartProvider,
        desired: bool,
        persist: F,
    ) -> Result<(), AutostartError>
    where
        F: FnOnce(&Config) -> Result<(), ConfigError>,
    {
        let mut staged = self.original.clone();
        staged.autostart = self.autostart;
        apply_autostart(provider, &mut staged, desired, persist)?;
        self.autostart = desired;
        self.original.autostart = desired;
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum SettingsValidationError {
    Interval(IntervalError),
    Hotkey(hotkey::ParseError),
    SpriteX(String),
    SpriteY(String),
    CardWidth(String),
}

impl std::fmt::Display for SettingsValidationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SettingsValidationError::Interval(e) => write!(f, "{e}"),
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

pub fn parse_interval_minutes(text: &str) -> Result<f64, IntervalError> {
    let trimmed = text.trim();
    let n: f64 = trimmed.parse().map_err(|_| IntervalError {
        input: trimmed.to_string(),
    })?;
    if !config::valid_interval_minutes(n) {
        return Err(IntervalError {
            input: trimmed.to_string(),
        });
    }
    Ok(n)
}

fn parse_points(text: &str) -> Option<i32> {
    let trimmed = text.trim();
    let digits = trimmed.strip_prefix('-').unwrap_or(trimmed);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    trimmed.parse().ok()
}

fn format_interval(n: f64) -> String {
    if n.fract() == 0.0 && n.is_finite() {
        format!("{}", n as i64)
    } else {
        format!("{n}")
    }
}

pub struct HotkeyRecorder {
    state: Recorder,
    hint: Option<String>,
    source: Option<Box<dyn EdgeSource>>,
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

    pub fn available(&self) -> bool {
        self.source.is_some()
    }

    pub fn is_armed(&self) -> bool {
        self.state == Recorder::Armed
    }

    pub fn hint(&self) -> Option<&str> {
        self.hint.as_deref()
    }

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

    pub fn pump(&mut self, focused: bool) -> Vec<Effect> {
        let mut effects = Vec::new();
        if self.was_focused && !focused {
            effects.push(self.feed(&Input::FocusLost));
        }
        self.was_focused = focused;

        let verdicts: Vec<Verdict> = match self.source.as_mut() {
            Some(source) => source
                .drain()
                .into_iter()
                .map(|e| source.classify(e))
                .collect(),
            None => return effects,
        };
        for verdict in verdicts {
            effects.push(self.feed(&Input::Press(verdict)));
        }
        effects
    }
}

pub type PersistFn = Box<dyn FnMut(&Config) -> Result<(), ConfigError> + Send>;

pub type SpriteCheckFn = Box<dyn Fn(&Path) -> Result<(), String> + Send>;

pub type ConflictFn = Box<dyn Fn(&Hotkey) -> Option<String> + Send>;

pub struct SettingsHost {
    pub provider: Box<dyn AutostartProvider>,
    pub persist: PersistFn,
    pub sprite_check: SpriteCheckFn,
    pub conflict: ConflictFn,
    #[cfg(not(target_arch = "wasm32"))]
    pub updates: UpdateHost,
}

#[cfg(not(target_arch = "wasm32"))]
pub type CheckFn = std::sync::Arc<dyn Fn() -> Result<crate::update::Release, String> + Send + Sync>;

#[cfg(not(target_arch = "wasm32"))]
pub type InstallFn =
    std::sync::Arc<dyn Fn(&crate::update::Release) -> Result<(), String> + Send + Sync>;

#[cfg(not(target_arch = "wasm32"))]
pub struct UpdateHost {
    pub current: String,
    pub action: crate::update::Action,
    pub check: CheckFn,
    pub install: InstallFn,
    pub open_page: Box<dyn Fn(&str) + Send>,
}

#[cfg(not(target_arch = "wasm32"))]
impl UpdateHost {
    pub fn offline() -> Self {
        Self {
            current: crate::update::CURRENT.to_string(),
            action: crate::update::ACTION,
            check: std::sync::Arc::new(|| Err("No update check in this build".to_string())),
            install: std::sync::Arc::new(|_| Err("No update check in this build".to_string())),
            open_page: Box::new(|_| {}),
        }
    }

    pub fn native(tray_thread: Option<u32>) -> Self {
        #[cfg(target_os = "windows")]
        let install: InstallFn = std::sync::Arc::new(move |release| {
            let exe =
                std::env::current_exe().map_err(|e| format!("Could not replace Nudge: {e}"))?;
            crate::update::install_release(release, &exe, tray_thread)
        });
        #[cfg(not(target_os = "windows"))]
        let install = {
            let _ = tray_thread;
            Self::offline().install
        };
        Self {
            current: crate::update::CURRENT.to_string(),
            action: crate::update::ACTION,
            check: std::sync::Arc::new(crate::update::fetch_latest),
            install,
            open_page: Box::new(crate::update::open_page),
        }
    }
}

impl SettingsHost {
    pub fn headless(provider: Box<dyn AutostartProvider>) -> Self {
        Self {
            provider,
            persist: Box::new(|_cfg: &Config| Ok(())),
            sprite_check: Box::new(|_path: &Path| Ok(())),
            conflict: Box::new(|_hk| None),
            #[cfg(not(target_arch = "wasm32"))]
            updates: UpdateHost::offline(),
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn native(
        config_path: std::path::PathBuf,
        initial_autostart: bool,
        tray_thread: Option<u32>,
    ) -> Result<Self, String> {
        let provider = crate::autostart::provider_for_current_exe(initial_autostart)?;
        Ok(Self {
            provider,
            persist: Box::new(move |c: &Config| config::save(&config_path, c)),
            sprite_check: Box::new(|path: &Path| {
                crate::sprite_png::check_sprite_file(path).map_err(|e| e.to_string())
            }),
            conflict: Box::new(crate::platform::hotkey_conflict),
            updates: UpdateHost::native(tray_thread),
        })
    }
}

pub fn save_form(
    form: &mut SettingsForm,
    sprite_check: &SpriteCheckFn,
    persist: &mut PersistFn,
) -> Result<Config, String> {
    let cfg = form.to_config().map_err(|e| e.to_string())?;
    if cfg.sprite.enabled {
        sprite_check(Path::new(&cfg.sprite.path))?;
    }
    persist(&cfg).map_err(|e| format!("Save failed: {e}"))?;
    form.mark_clean(cfg.clone());
    Ok(cfg)
}

pub fn apply_effect(effect: &Effect, form: &mut SettingsForm) -> bool {
    match effect {
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

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn pick_sprite_file() -> Option<String> {
    rfd::FileDialog::new()
        .set_title("Choose a sprite")
        .add_filter("PNG image", &["png"])
        .pick_file()
        .map(|p| p.display().to_string())
}

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

pub struct SettingsApp {
    form: SettingsForm,
    provider: Box<dyn AutostartProvider>,
    persist: PersistFn,
    sprite_check: SpriteCheckFn,
    conflict: ConflictFn,
    recorder: HotkeyRecorder,
    banner: Option<String>,
    saved: bool,
    quit_requested: bool,
    #[cfg(not(target_arch = "wasm32"))]
    updates: UpdateHost,
    #[cfg(not(target_arch = "wasm32"))]
    update_row: crate::update::Row,
    #[cfg(not(target_arch = "wasm32"))]
    update_job: Option<std::sync::mpsc::Receiver<crate::update::Input>>,
}

impl SettingsApp {
    pub fn new(cc: &eframe::CreationContext<'_>, config: Config, host: SettingsHost) -> Self {
        // egui's set_visuals writes only the style of the active theme, and under
        // ThemePreference::System the active theme is the OS theme from raw input.
        // proof: https://docs.rs/crate/egui/0.34.2/source/src/context.rs#2235
        cc.egui_ctx.set_theme(eframe::egui::ThemePreference::Dark);
        cc.egui_ctx.set_visuals(eframe::egui::Visuals::dark());
        #[cfg(not(target_arch = "wasm32"))]
        crate::platform::install_native_fonts(&cc.egui_ctx);
        #[cfg(not(target_arch = "wasm32"))]
        let source = crate::platform::new_edge_source(&cc.egui_ctx);
        #[cfg(target_arch = "wasm32")]
        let source = None;
        Self::with_source(config, host, source)
    }

    pub fn with_source(
        config: Config,
        host: SettingsHost,
        source: Option<Box<dyn EdgeSource>>,
    ) -> Self {
        let SettingsHost {
            provider,
            persist,
            sprite_check,
            conflict,
            #[cfg(not(target_arch = "wasm32"))]
            updates,
        } = host;
        Self {
            form: SettingsForm::from_config(config),
            provider,
            persist,
            sprite_check,
            conflict,
            recorder: HotkeyRecorder::new(source),
            banner: None,
            saved: false,
            quit_requested: false,
            #[cfg(not(target_arch = "wasm32"))]
            updates,
            #[cfg(not(target_arch = "wasm32"))]
            update_row: crate::update::Row::Idle,
            #[cfg(not(target_arch = "wasm32"))]
            update_job: None,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn feed_update(&mut self, input: crate::update::Input, ctx: &egui::Context) {
        use crate::update::{Effect, Input};
        let (row, effect) = crate::update::step(
            &self.update_row,
            input,
            &self.updates.current,
            self.updates.action,
        );
        self.update_row = row;
        match effect {
            Effect::Nothing => {}
            Effect::Check => {
                let check = std::sync::Arc::clone(&self.updates.check);
                self.spawn_update_job(ctx, move || Input::Checked(check()));
            }
            Effect::Install(release) => {
                let install = std::sync::Arc::clone(&self.updates.install);
                self.spawn_update_job(ctx, move || Input::Installed(install(&release)));
            }
            Effect::OpenPage(url) => (self.updates.open_page)(&url),
            Effect::Close => self.quit_requested = true,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn spawn_update_job(
        &mut self,
        ctx: &egui::Context,
        job: impl FnOnce() -> crate::update::Input + Send + 'static,
    ) {
        let (tx, rx) = std::sync::mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let _ = tx.send(job());
            ctx.request_repaint();
        });
        self.update_job = Some(rx);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn poll_update_job(&mut self, ctx: &egui::Context) {
        let answer = self.update_job.as_ref().map(|rx| rx.try_recv());
        match answer {
            Some(Ok(input)) => {
                self.update_job = None;
                self.feed_update(input, ctx);
            }
            Some(Err(std::sync::mpsc::TryRecvError::Disconnected)) => self.update_job = None,
            _ => {}
        }
    }

    fn apply_recorder_effects(&mut self, focused: bool) {
        for effect in self.recorder.pump(focused) {
            self.apply_recorder_effect(&effect);
        }
    }

    fn apply_recorder_effect(&mut self, effect: &Effect) {
        if apply_effect(effect, &mut self.form) {
            self.banner = None;
        }
    }

    fn save(&mut self) {
        if !self.form.is_dirty() {
            return;
        }
        match save_form(&mut self.form, &self.sprite_check, &mut self.persist) {
            Ok(saved) => {
                self.saved = true;
                self.banner = hotkey::parse(&saved.hotkey)
                    .ok()
                    .and_then(|hk| (self.conflict)(&hk))
                    .map(|warning| format!("This combination is {warning}"));
            }
            Err(line) => self.banner = Some(line),
        }
    }

    fn apply_autostart_toggle(&mut self, desired: bool) {
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

pub const WINDOW_SIZE: [f32; 2] = [560.0, 625.0];

#[cfg(not(target_arch = "wasm32"))]
const UPDATE_STATUS_WIDTH: f32 = 270.0;

pub const MARGIN: i8 = 20;
const ROW_PAD: i8 = 12;
const ROW_HEIGHT: f32 = 44.0;
const GROUP_RADIUS: u8 = 10;
const HAIRLINE: f32 = 1.0;
const FOOTNOTE_GAP: f32 = 8.0;
const FOOTNOTE_SIZE: f32 = 12.0;
const GROUP_GAP: f32 = 20.0;
const BANNER_HEIGHT: f32 = 64.0;
const BANNER_COLOR: egui::Color32 = egui::Color32::from_rgb(251, 191, 36);
const HOTKEY_FIELD_WIDTH: f32 = 200.0;
const NUMBER_FIELD_WIDTH: f32 = 72.0;
const BUTTON_SIZE: egui::Vec2 = egui::vec2(72.0, 30.0);
const BUTTON_GAP: f32 = 12.0;
const SAVE_FILL: egui::Color32 = egui::Color32::from_rgb(79, 70, 229);

fn group(ui: &mut egui::Ui, rows: impl FnOnce(&mut egui::Ui)) {
    let visuals = ui.visuals().clone();
    egui::Frame::new()
        .fill(visuals.faint_bg_color)
        .stroke(egui::Stroke::new(
            HAIRLINE,
            visuals.widgets.noninteractive.bg_stroke.color,
        ))
        .corner_radius(GROUP_RADIUS)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            rows(ui);
        });
}

fn footnote(ui: &mut egui::Ui, text: &str) {
    ui.add_space(FOOTNOTE_GAP);
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(ROW_PAD, 0))
        .show(ui, |ui| {
            ui.add(egui::Label::new(egui::RichText::new(text).size(FOOTNOTE_SIZE).weak()).wrap());
        });
}

fn hairline(ui: &mut egui::Ui) {
    let (rect, _) = ui.allocate_exact_size(
        egui::vec2(ui.available_width(), HAIRLINE),
        egui::Sense::hover(),
    );
    ui.painter().hline(
        rect.x_range(),
        rect.center().y,
        egui::Stroke::new(
            HAIRLINE,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ),
    );
}

fn dimmed(ui: &mut egui::Ui, dim: bool, contents: impl FnOnce(&mut egui::Ui)) {
    ui.scope(|ui| {
        if dim {
            ui.multiply_opacity(ui.visuals().disabled_alpha());
        }
        contents(ui);
    });
}

fn row_with(ui: &mut egui::Ui, text: &str, rest: impl FnOnce(&mut egui::Ui, &egui::Response)) {
    egui::Frame::new()
        .inner_margin(egui::Margin::symmetric(ROW_PAD, 0))
        .show(ui, |ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(ui.available_width(), ROW_HEIGHT),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.set_min_height(ROW_HEIGHT);
                    let label = ui.label(text);
                    rest(ui, &label);
                },
            );
        });
}

fn row(ui: &mut egui::Ui, text: &str, id_salt: &str, controls: impl FnOnce(&mut egui::Ui)) {
    row_with(ui, text, |ui, _| {
        right_aligned(ui, id_salt, ROW_HEIGHT, controls)
    });
}

fn checkbox_row(ui: &mut egui::Ui, text: &str, checked: &mut bool) -> bool {
    let mut changed = false;
    row_with(ui, text, |ui, label| {
        right_aligned(ui, text, ROW_HEIGHT, |ui| {
            let checkbox = ui
                .add(egui::Checkbox::without_text(checked))
                .labelled_by(label.id);
            ui.ctx()
                .accesskit_node_builder(checkbox.id, |node| node.set_label(text));
            changed = checkbox.changed();
        });
    });
    changed
}

fn right_aligned(
    ui: &mut egui::Ui,
    id_salt: &str,
    height: f32,
    contents: impl FnOnce(&mut egui::Ui),
) {
    let id = width_id(id_salt);
    let known = remembered_width(ui, id);
    let band = egui::vec2(ui.available_width(), height);
    ui.allocate_ui_with_layout(
        band,
        egui::Layout::right_to_left(egui::Align::Center),
        |ui| {
            let width = known.unwrap_or_else(|| ui.available_width());
            let used = ui
                .allocate_ui_with_layout(
                    egui::vec2(width, height),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_min_height(height);
                        contents(ui);
                    },
                )
                .response
                .rect
                .width();
            remember_width(ui, id, used);
        },
    );
}

fn width_id(id_salt: &str) -> egui::Id {
    egui::Id::new(("settings width", id_salt))
}

fn remembered_width(ui: &egui::Ui, id: egui::Id) -> Option<f32> {
    ui.data(|d| d.get_temp::<f32>(id))
}

fn remember_width(ui: &egui::Ui, id: egui::Id, width: f32) {
    if remembered_width(ui, id).is_none_or(|w| (w - width).abs() > 0.5) {
        ui.data_mut(|d| d.insert_temp(id, width));
        ui.ctx().request_discard("settings row width");
    }
}

impl eframe::App for SettingsApp {
    fn ui(&mut self, root_ui: &mut eframe::egui::Ui, _frame: &mut eframe::Frame) {
        use eframe::egui;

        let ctx_owned = root_ui.ctx().clone();
        let ctx = &ctx_owned;

        let before = self.form.clone();

        let focused = ctx.input(|i| i.viewport().focused).unwrap_or(true);
        self.apply_recorder_effects(focused);

        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            self.quit_requested = true;
        }
        if ctx.input(|i| i.viewport().close_requested()) {
            self.quit_requested = true;
        }

        let armed = self.recorder.is_armed();
        let mut autostart_toggle: Option<bool> = None;
        let mut save_clicked = false;
        #[cfg(not(target_arch = "wasm32"))]
        let mut update_pressed = false;
        #[cfg(not(target_arch = "wasm32"))]
        self.poll_update_job(ctx);

        egui::Frame::new()
            .inner_margin(egui::Margin::same(MARGIN))
            .show(root_ui, |ui| {
                ui.spacing_mut().item_spacing.y = 0.0;

                group(ui, |ui| {
                    row(ui, "Global hotkey", "hotkey", |ui| {
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
                                    .desired_width(HOTKEY_FIELD_WIDTH)
                                    .hint_text(hotkey::format_native(&hotkey::default_hotkey())),
                            );
                        }
                        if self.recorder.available() {
                            let label = if armed { "Cancel" } else { "Record" };
                            if ui.button(label).clicked() {
                                let input = if armed { Input::Cancel } else { Input::Record };
                                let effect = self.recorder.feed(&input);
                                self.apply_recorder_effect(&effect);
                            }
                        }
                    });
                });
                footnote(ui, self.recorder.hint().unwrap_or(hotkey::VOCABULARY));
                ui.add_space(GROUP_GAP);

                group(ui, |ui| {
                    row(ui, "Default interval", "interval", |ui| {
                        ui.add(
                            egui::TextEdit::singleline(&mut self.form.interval_text)
                                .id(egui::Id::new(ID_INTERVAL))
                                .desired_width(NUMBER_FIELD_WIDTH)
                                .hint_text("10"),
                        );
                        ui.label(egui::RichText::new("min").weak());
                    });
                    hairline(ui);
                    #[cfg(target_os = "macos")]
                    let label = "Open at login";
                    #[cfg(not(target_os = "macos"))]
                    let label = "Launch with Windows";
                    let mut autostart = self.form.autostart;
                    if checkbox_row(ui, label, &mut autostart) {
                        autostart_toggle = Some(autostart);
                    }
                });
                ui.add_space(GROUP_GAP);

                group(ui, |ui| {
                    checkbox_row(ui, "Sprite", &mut self.form.sprite_enabled);
                    let off = !self.form.sprite_enabled;
                    hairline(ui);
                    dimmed(ui, off, |ui| {
                        row_with(ui, "PNG", |ui, _| {
                            let field_width = ui.available_width()
                                - remembered_width(ui, width_id("browse"))
                                    .map_or(0.0, |w| w + ui.spacing().item_spacing.x);
                            ui.add(
                                egui::TextEdit::singleline(&mut self.form.sprite_path)
                                    .id(egui::Id::new(ID_SPRITE_PATH))
                                    .desired_width(field_width)
                                    .hint_text("Path to a PNG with a transparent background"),
                            );
                            #[cfg(any(target_os = "windows", target_os = "macos"))]
                            {
                                let browse = ui.add_enabled(!armed, egui::Button::new("Browse…"));
                                remember_width(ui, width_id("browse"), browse.rect.width());
                                if browse.clicked()
                                    && let Some(picked) = pick_sprite_file()
                                {
                                    self.form.sprite_path = picked;
                                }
                            }
                        })
                    });
                    hairline(ui);
                    dimmed(ui, off, |ui| {
                        row(ui, "Position (pt)", "position", |ui| {
                            for (name, text, id, hint) in [
                                (
                                    "X",
                                    &mut self.form.sprite_x_text,
                                    ID_SPRITE_X,
                                    "0".to_string(),
                                ),
                                (
                                    "Y",
                                    &mut self.form.sprite_y_text,
                                    ID_SPRITE_Y,
                                    "0".to_string(),
                                ),
                                (
                                    "Card width",
                                    &mut self.form.card_width_text,
                                    ID_CARD_WIDTH,
                                    crate::config::DEFAULT_CARD_WIDTH.to_string(),
                                ),
                            ] {
                                ui.label(egui::RichText::new(name).weak());
                                ui.add(
                                    egui::TextEdit::singleline(text)
                                        .id(egui::Id::new(id))
                                        .desired_width(NUMBER_FIELD_WIDTH)
                                        .hint_text(hint),
                                );
                            }
                        })
                    });
                });
                ui.add_space(GROUP_GAP);

                #[cfg(not(target_arch = "wasm32"))]
                {
                    let version = format!("Nudge {}", self.updates.current);
                    group(ui, |ui| {
                        row(ui, &version, "update", |ui| {
                            let status = self.update_row.status();
                            if !status.is_empty() {
                                ui.scope(|ui| {
                                    ui.set_max_width(UPDATE_STATUS_WIDTH);
                                    ui.add(
                                        egui::Label::new(egui::RichText::new(status).weak())
                                            .truncate(),
                                    );
                                });
                            }
                            let (label, enabled) = self.update_row.button(self.updates.action);
                            update_pressed =
                                ui.add_enabled(enabled, egui::Button::new(label)).clicked();
                        });
                    });
                    footnote(ui, crate::update::FOOTNOTE);
                    ui.add_space(GROUP_GAP);
                }

                let (line, _) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), BANNER_HEIGHT),
                    egui::Sense::hover(),
                );
                if let Some(msg) = &self.banner {
                    let mut ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(line)
                            .layout(egui::Layout::bottom_up(egui::Align::Min)),
                    );
                    ui.set_clip_rect(line.intersect(ui.clip_rect()));
                    ui.add(egui::Label::new(egui::RichText::new(msg).color(BANNER_COLOR)).wrap());
                }
                ui.add_space(GROUP_GAP);

                right_aligned(ui, "buttons", BUTTON_SIZE.y, |ui| {
                    ui.spacing_mut().item_spacing.x = BUTTON_GAP;
                    if self.saved {
                        ui.label(egui::RichText::new("Saved").weak());
                    }
                    if ui
                        .add(egui::Button::new("Cancel").min_size(BUTTON_SIZE))
                        .clicked()
                    {
                        self.quit_requested = true;
                    }
                    save_clicked = ui
                        .add_enabled(
                            self.form.is_dirty(),
                            egui::Button::new(
                                egui::RichText::new("Save").color(egui::Color32::WHITE),
                            )
                            .fill(SAVE_FILL)
                            .min_size(BUTTON_SIZE),
                        )
                        .clicked();
                });
            });

        if let Some(desired) = autostart_toggle {
            self.apply_autostart_toggle(desired);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if update_pressed {
            self.feed_update(crate::update::Input::Press, ctx);
        }

        if self.form != before {
            self.saved = false;
        }

        let return_in_the_form = !armed
            && ctx.input(|i| i.key_pressed(egui::Key::Enter))
            && ctx
                .memory(|m| m.focused())
                .is_none_or(|id| FIELD_IDS.iter().any(|name| egui::Id::new(name) == id));

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
        let f = SettingsForm::from_config(cfg_with("  alt + j  ", 10.0, false));
        assert_eq!(f.hotkey, native("Alt+J"));
        assert!(
            !f.is_dirty(),
            "re-spelling on load is not an edit the user made"
        );
    }

    #[test]
    fn an_unparseable_stored_hotkey_is_shown_exactly_as_stored() {
        let f = SettingsForm::from_config(cfg_with("Ctrl+Hoem", 10.0, false));
        assert_eq!(f.hotkey, "Ctrl+Hoem");
    }

    #[test]
    fn fractional_interval_renders_without_trailing_zero_noise() {
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
        for bad in ["", "abc", "0", "-5", "NaN", "Infinity"] {
            assert!(
                parse_interval_minutes(bad).is_err(),
                "{bad:?} should be rejected"
            );
        }
    }

    #[test]
    fn parse_interval_minutes_holds_the_interval_bound() {
        assert_eq!(parse_interval_minutes("1000000").unwrap(), 1_000_000.0);
        for bad in ["1000000.0000001", "2e17", "1e20"] {
            assert_eq!(
                parse_interval_minutes(bad),
                Err(IntervalError {
                    input: bad.to_string()
                }),
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
        let mut f = SettingsForm::from_config(Config::default());
        f.hotkey = "option+cmd+j".to_string();
        let saved = f.to_config().unwrap();
        f.mark_clean(saved.clone());
        assert_eq!(f.hotkey, saved.hotkey);
        assert!(!f.is_dirty(), "a just-saved form is not dirty");
    }

    #[test]
    fn toggle_autostart_success_flips_form_and_persists() {
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
    fn a_platform_with_no_key_source_shows_no_record_button() {
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
    fn the_tray_thread_is_the_number_after_its_flag() {
        assert_eq!(
            parse_tray_thread_arg(["--settings", "--tray-thread", "4242"]),
            Some(4242)
        );
        assert_eq!(parse_tray_thread_arg(["--settings"]), None);
        assert_eq!(parse_tray_thread_arg(["--tray-thread"]), None);
        assert_eq!(parse_tray_thread_arg(["--tray-thread", "x"]), None);
    }

    #[test]
    fn toggle_autostart_persist_failure_surfaces_but_os_still_changed() {
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
