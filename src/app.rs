use eframe::egui;

use crate::config::SpriteConfig;
use crate::journal::{self, JournalSink};
use crate::layout::{self, PopupLayout, ScreenGeometry};
use crate::nudge_state::{self, Action, TriggerSource};
#[cfg(target_arch = "wasm32")]
use crate::timer::Timer;
use crate::word_jump;

pub const CLEAR_COLOR_TRANSPARENT: [f32; 4] = [0.0, 0.0, 0.0, 0.0];

/// Placeholder / hint text colour. egui's dark default is ~gray(120), which
/// washes out against the translucent card. Bumped to gray(170) so hints
/// like "Что я делаю?" (the popup's question copy — kept Russian as the
/// product placeholders) are comfortably legible. Must be applied via
/// `visuals.weak_text_color`: egui 0.34's TextEdit overwrites any per-hint
/// RichText color with `visuals.weak_text_color()` (widgets/text_edit/
/// builder.rs:586), and the computed default — text color gamma-multiplied
/// by `weak_text_alpha` — renders the hints nearly invisible (~peak 58)
/// over the dark card.
const HINT_TEXT_COLOR: egui::Color32 = egui::Color32::from_gray(170);

pub(crate) const CARD_RADIUS: u8 = 16;

const FOCUS_FIELD_INSET: egui::Vec2 = egui::vec2(10.0, 5.0);
const FOCUS_FIELD_RADIUS: u8 = 8;

#[cfg(target_os = "macos")]
const FOCUS_RETRY_WINDOW: std::time::Duration = std::time::Duration::from_millis(500);

#[cfg(target_os = "macos")]
const HIDDEN_WINDOW_SIZE: egui::Vec2 = egui::vec2(1.0, 1.0);

#[cfg(target_os = "macos")]
const CARD_FILL_ALPHA: u8 = 120;
#[cfg(not(target_os = "macos"))]
const CARD_FILL_ALPHA: u8 = 220;

#[cfg(target_os = "macos")]
const CARD_SHADOW: egui::epaint::Shadow = egui::epaint::Shadow::NONE;
#[cfg(not(target_os = "macos"))]
const CARD_SHADOW: egui::epaint::Shadow = egui::epaint::Shadow {
    offset: [0, 12],
    blur: 40,
    spread: 0,
    color: egui::Color32::from_rgba_premultiplied(0, 0, 0, 160),
};

const CARD_STROKE: egui::Stroke = egui::Stroke {
    width: 1.0,
    color: egui::Color32::from_rgba_premultiplied(60, 60, 60, 60),
};

pub struct LoadedSprite {
    image: Option<egui::ColorImage>,
    tex: Option<egui::TextureHandle>,
    px: [u32; 2],
    cfg: SpriteConfig,
}

#[cfg(not(target_arch = "wasm32"))]
impl LoadedSprite {
    fn from_config(cfg: &SpriteConfig) -> Option<Self> {
        match crate::sprite_png::load_sprite(cfg)? {
            Ok(decoded) => Some(Self::decoded(decoded, cfg.clone())),
            Err(reason) => {
                eprintln!("[nudge] sprite not shown: {reason}");
                None
            }
        }
    }

    fn decoded(sprite: crate::sprite_png::DecodedSprite, cfg: SpriteConfig) -> Self {
        let image = egui::ColorImage::from_rgba_unmultiplied(
            [sprite.px[0] as usize, sprite.px[1] as usize],
            &sprite.rgba,
        );
        Self {
            image: Some(image),
            tex: None,
            px: sprite.px,
            cfg,
        }
    }
}

impl LoadedSprite {
    fn texture(&mut self, ctx: &egui::Context) -> Option<egui::TextureId> {
        if self.tex.is_none()
            && ctx.input(|i| i.viewport().visible().unwrap_or(true))
            && let Some(image) = self.image.take()
        {
            self.tex = Some(ctx.load_texture("nudge_sprite", image, egui::TextureOptions::NEAREST));
        }
        self.tex.as_ref().map(|t| t.id())
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Placement {
    pub geometry: ScreenGeometry,
    pub ppp: f32,
    pub sprite: Option<([u32; 2], (i32, i32), u32)>,
}

pub fn placement_is_stale(placed: Option<&Placement>, now: &Placement) -> bool {
    placed != Some(now)
}

pub fn sprite_apply(pending: Option<&SpriteConfig>, popup_visible: bool) -> Option<&SpriteConfig> {
    pending.filter(|_| !popup_visible)
}

pub struct NudgeApp {
    doing: String,
    bullshit: String,
    next_minutes: String,
    focus_first: bool,
    first_frame: bool,

    #[cfg(target_arch = "wasm32")]
    timer: Timer,
    journal: JournalSink,
    trigger_source: TriggerSource,
    popup_visible: bool,
    error_message: Option<String>,
    card_rect: Option<egui::Rect>,
    sprite: Option<LoadedSprite>,
    retired_textures: Vec<egui::TextureHandle>,
    #[cfg(not(target_arch = "wasm32"))]
    pending_sprite: Option<SpriteConfig>,
    #[cfg(not(target_arch = "wasm32"))]
    deadline_delivered: bool,
    geometry_override: Option<ScreenGeometry>,
    placed: Option<Placement>,
    #[cfg(target_os = "macos")]
    window_shrunk: bool,
    #[cfg(target_os = "macos")]
    order_out_after_pass: Option<u64>,
    layout: Option<PopupLayout>,
    #[cfg(target_arch = "wasm32")]
    pill_rect: Option<egui::Rect>,
    armed: bool,
    #[cfg(target_os = "macos")]
    shown_at: Option<std::time::Instant>,
}

impl NudgeApp {
    pub fn new(cc: &eframe::CreationContext<'_>, default_minutes: f64) -> Self {
        let mut visuals = egui::Visuals::dark();
        visuals.panel_fill = egui::Color32::TRANSPARENT;
        visuals.window_fill = egui::Color32::TRANSPARENT;
        visuals.widgets.noninteractive.fg_stroke.color = HINT_TEXT_COLOR;
        visuals.widgets.inactive.fg_stroke.color = HINT_TEXT_COLOR;
        visuals.weak_text_color = Some(HINT_TEXT_COLOR);
        // Pin the theme BEFORE installing the visuals. set_visuals only
        // writes into the currently-active theme's style, and with the
        // default ThemePreference::System egui flips to the OS theme
        // between construction and frame #1 — on a light-themed OS every
        // override above would sit unused in the dark style while the
        // popup renders with stock light visuals (observed in the browser
        // build: theme=Dark in new(), theme=Light by the first frame).
        // The spotlight card is dark by design, so the theme is not a
        // user preference here.
        cc.egui_ctx.set_theme(egui::ThemePreference::Dark);
        cc.egui_ctx.set_visuals(visuals);

        // Force frame #1 ourselves. When the process starts without window
        // activation (autostart at logon, spawn from a background shell) the
        // OS delivers no input events to the fresh window, and the event
        // loop never paints on its own: `ui()` never runs, so the window is
        // never adopted by the platform (its Show/hotkey wakeups need it)
        // and the launch interval is never armed. Observed live on Windows:
        // launch unfocused → no frame until the first real input event hits
        // the window.
        cc.egui_ctx.request_repaint();

        Self {
            doing: String::new(),
            bullshit: String::new(),
            next_minutes: default_minutes.to_string(),
            focus_first: true,
            first_frame: true,
            #[cfg(target_arch = "wasm32")]
            timer: Timer::frozen(),
            journal: journal::default_sink(),
            trigger_source: TriggerSource::Timer,
            popup_visible: false,
            error_message: None,
            card_rect: None,
            sprite: None,
            retired_textures: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            pending_sprite: None,
            #[cfg(not(target_arch = "wasm32"))]
            deadline_delivered: false,
            geometry_override: None,
            placed: None,
            #[cfg(target_os = "macos")]
            window_shrunk: false,
            #[cfg(target_os = "macos")]
            order_out_after_pass: None,
            layout: None,
            #[cfg(target_arch = "wasm32")]
            pill_rect: None,
            armed: false,
            #[cfg(target_os = "macos")]
            shown_at: None,
        }
    }

    pub fn with_journal_sink(mut self, sink: JournalSink) -> Self {
        self.journal = sink;
        self
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_sprite_config(mut self, cfg: &SpriteConfig) -> Self {
        self.sprite = LoadedSprite::from_config(cfg);
        self
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn with_sprite(
        mut self,
        sprite: crate::sprite_png::DecodedSprite,
        cfg: SpriteConfig,
    ) -> Self {
        self.sprite = Some(LoadedSprite::decoded(sprite, cfg));
        self
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn set_pending_sprite(&mut self, cfg: SpriteConfig) {
        self.pending_sprite = Some(cfg);
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub fn deliver_deadline(&mut self) {
        self.deadline_delivered = true;
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn apply_pending_sprite(&mut self) {
        if let Some(cfg) = crate::platform::take_pending_sprite() {
            self.pending_sprite = Some(cfg);
        }
        let Some(cfg) = sprite_apply(self.pending_sprite.as_ref(), self.popup_visible).cloned()
        else {
            return;
        };
        self.pending_sprite = None;
        if let Some(tex) = self.sprite.take().and_then(|s| s.tex) {
            self.retired_textures.push(tex);
        }
        self.sprite = LoadedSprite::from_config(&cfg);
        self.placed = None;
    }

    pub fn with_screen_geometry(mut self, geometry: ScreenGeometry) -> Self {
        self.geometry_override = Some(geometry);
        self
    }

    fn place_window(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        #[cfg(target_os = "macos")]
        if self.window_shrunk {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        let screen = self
            .geometry_override
            .or_else(|| crate::platform::screen_geometry(ctx, frame));
        #[cfg(target_arch = "wasm32")]
        let screen = {
            let _ = frame;
            self.geometry_override
        };
        let Some(geometry) = screen else {
            self.layout = None;
            self.placed = None;
            return;
        };

        let now = Placement {
            geometry,
            ppp: ctx.pixels_per_point(),
            sprite: self
                .sprite
                .as_ref()
                .map(|s| (s.px, (s.cfg.x, s.cfg.y), s.cfg.card_width)),
        };
        if !placement_is_stale(self.placed.as_ref(), &now) {
            return;
        }

        // TODO: a second monitor is not shielded.
        // Size first: winit converts the position to AppKit's bottom-left
        // origin with the window's current height.
        ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(geometry.work.size()));
        ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(geometry.work.min));

        let placement = self
            .sprite
            .as_ref()
            .map(|s| layout::placement_of(&s.cfg, s.px, now.ppp));
        self.layout = Some(layout::shield_layout(&geometry, placement));
        self.placed = Some(now);
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn restore_window(&mut self, ctx: &egui::Context, frame: &eframe::Frame) {
        #[cfg(target_os = "macos")]
        {
            self.window_shrunk = false;
            self.order_out_after_pass = None;
        }
        self.place_window(ctx, frame);
    }

    #[cfg(target_os = "macos")]
    fn order_out_if_owed(&mut self, ctx: &egui::Context) {
        let Some(pass) = self.order_out_after_pass else {
            return;
        };
        if ctx.cumulative_pass_nr() > pass {
            self.order_out_after_pass = None;
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
        } else {
            ctx.request_repaint();
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn now_timestamp() -> String {
        chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false)
    }

    #[cfg(target_arch = "wasm32")]
    fn now_timestamp() -> String {
        let date = js_sys::Date::new_0();
        let offset_total = -(date.get_timezone_offset() as i32);
        let sign = if offset_total >= 0 { '+' } else { '-' };
        let offset_h = offset_total.abs() / 60;
        let offset_m = offset_total.abs() % 60;
        format!(
            "{}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}{}{:02}:{:02}",
            date.get_full_year(),
            date.get_month() + 1,
            date.get_date(),
            date.get_hours(),
            date.get_minutes(),
            date.get_seconds(),
            date.get_milliseconds(),
            sign,
            offset_h,
            offset_m
        )
    }

    fn show_popup(&mut self, ctx: &egui::Context, source: TriggerSource) {
        self.trigger_source = source;
        self.popup_visible = true;
        self.focus_first = true;

        // Tell egui itself the viewport is visible again — without this
        // counterpart to the hide_popup `Visible(false)` send, egui keeps
        // believing the window is hidden and skips painting.
        #[cfg(not(target_arch = "wasm32"))]
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));

        #[cfg(not(target_arch = "wasm32"))]
        crate::platform::set_popup_visible(true);

        #[cfg(not(target_arch = "wasm32"))]
        crate::platform::focus_popup(ctx);

        self.armed = self.current_foreground_matches(ctx);
        #[cfg(target_os = "macos")]
        {
            self.shown_at = Some(std::time::Instant::now());
        }
    }

    fn current_foreground_matches(&self, ctx: &egui::Context) -> bool {
        #[cfg(not(target_arch = "wasm32"))]
        {
            crate::platform::popup_is_foreground(ctx)
        }
        #[cfg(target_arch = "wasm32")]
        {
            let _ = ctx;
            web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.has_focus().ok())
                .unwrap_or(false)
        }
    }

    /// Arm the focus-loss check once the popup is observably focused, for the
    /// platform whose activation call cannot answer synchronously.
    ///
    /// # Why a latch and not a snapshot, on macOS
    ///
    /// `show_popup` runs from `ui()`, and `ui()` cannot run in the same frame
    /// as the reveal that preceded it: eframe samples `is_visible` at the top
    /// of the frame, before `logic()` (and therefore before
    /// `platform::macos::tick` reveals anything), then gates `app.ui` on that
    /// sample. So by the time `show_popup` takes its snapshot, the
    /// `activateIgnoringOtherApps` from the reveal is one or more frames old —
    /// but `show_popup` has *also* just called `focus_popup` again, microseconds
    /// earlier, and that call is a request to the window server which no read
    /// on this thread reflects on return. `NSApplication::isActive` has the
    /// same property; it is not a stronger reading of the same moment, it is
    /// the same unanswerable question asked in objc.
    ///
    /// Whether the *earlier* activation has landed by then depends on the order
    /// in which AppKit delivers the occlusion change that makes the window
    /// visible and the key-window change that gives it the keyboard — which is
    /// not something this code gets to decide, and not something a locked
    /// screen can measure. A snapshot is therefore a coin flip, and a coin flip
    /// that lands wrong reproduces the original bug with a plausible-looking
    /// fix in place.
    ///
    /// A latch does not need the answer. Whichever frame the window becomes
    /// key on — the one running `show_popup` or a later one — arms us then, and
    /// the focus event that makes it key is itself a repaint, so the frame
    /// exists. A window that never becomes key never arms, which is precisely
    /// what spec §4 asks for.
    ///
    /// No-op everywhere else: Windows and WASM already had a true answer in
    /// `show_popup`, and re-deciding it per frame would change shipped
    /// behaviour on a platform this port cannot test.
    fn latch_arming(&mut self, focused_now: bool) {
        let _ = focused_now;

        #[cfg(target_os = "macos")]
        if focused_now {
            self.armed = true;
        }
    }

    fn retry_focus_until_armed(&mut self, ctx: &egui::Context) {
        let _ = ctx;
        #[cfg(target_os = "macos")]
        if !self.armed
            && ctx.input(|i| i.viewport().focused).is_some()
            && self
                .shown_at
                .is_some_and(|t| t.elapsed() < FOCUS_RETRY_WINDOW)
        {
            crate::platform::focus_popup(ctx);
            ctx.request_repaint_after(std::time::Duration::from_millis(30));
        }
    }

    fn hide_popup(&mut self, ctx: &egui::Context, action: Action) {
        let outcome = match nudge_state::decide_close(
            self.trigger_source,
            action,
            &self.doing,
            &self.bullshit,
            &self.next_minutes,
        ) {
            Ok(o) => o,
            Err(e) => {
                self.error_message = Some(e.to_string());
                return;
            }
        };

        if let Some(minutes) = outcome.journal_minutes {
            let event = journal::new_submitted_event(
                Self::now_timestamp(),
                self.trigger_source,
                self.doing.clone(),
                self.bullshit.clone(),
                minutes,
            );
            if let Err(e) = (self.journal)(&event) {
                self.error_message = Some(e.to_string());
                return;
            }
        }

        self.error_message = None;

        if outcome.clear_form {
            self.doing.clear();
            self.bullshit.clear();
        }
        self.enter_waiting(ctx, outcome.reset_timer);
    }

    #[cfg_attr(target_arch = "wasm32", expect(unused_variables))]
    fn enter_waiting(&mut self, ctx: &egui::Context, interval: Option<std::time::Duration>) {
        self.focus_first = true;
        self.popup_visible = false;
        self.armed = false;
        self.card_rect = None;

        #[cfg(not(target_arch = "wasm32"))]
        self.withdraw_window(ctx);

        if let Some(interval) = interval {
            self.arm_timer(interval);
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            crate::platform::set_popup_visible(false);
            self.apply_pending_sprite();
        }
    }

    fn arm_timer(&mut self, interval: std::time::Duration) {
        #[cfg(not(target_arch = "wasm32"))]
        crate::platform::set_timer_state(crate::platform::ArmedTimer {
            deadline: std::time::Instant::now() + interval,
            duration: interval,
        });
        #[cfg(target_arch = "wasm32")]
        self.timer.reset(interval);
    }

    /// Take the native window off screen. On macOS a placed window is shrunk
    /// to one point in this pass and ordered out in the next: AppKit resizes
    /// the presentation surfaces only of a window that is still on screen,
    /// and the pass in between paints one transparent 1×1 frame nobody sees.
    #[cfg(not(target_arch = "wasm32"))]
    fn withdraw_window(&mut self, ctx: &egui::Context) {
        #[cfg(target_os = "macos")]
        if self.placed.take().is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(HIDDEN_WINDOW_SIZE));
            self.window_shrunk = true;
            self.order_out_after_pass = Some(ctx.cumulative_pass_nr());
            ctx.request_repaint();
            return;
        }

        // Tell egui the viewport is hidden. Without this, egui's event
        // loop keeps pumping/scheduling for the "visible" viewport even
        // when the platform hides the native window directly — winit then
        // burns a full core on the main thread despite update() never being
        // called.
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
    }

    fn card_frame() -> egui::Frame {
        egui::Frame::NONE
            .fill(egui::Color32::from_rgba_unmultiplied(
                24,
                24,
                27,
                CARD_FILL_ALPHA,
            ))
            .corner_radius(egui::CornerRadius::same(CARD_RADIUS))
            .shadow(CARD_SHADOW)
            .inner_margin(egui::Margin::ZERO)
    }

    fn draw_card(&mut self, ctx: &egui::Context) {
        let card = self
            .layout
            .map_or_else(|| layout::card_in_box(ctx.content_rect()), |l| l.card);
        let card_width = card.width();

        let inner = egui::Area::new(egui::Id::new("nudge_card"))
            .fixed_pos(card.min)
            .interactable(false)
            .show(ctx, |ui| {
                ui.set_width(card_width);
                Self::card_frame()
                    .show(ui, |ui| {
                        ui.set_width(card_width);
                        self.draw_form(ui);
                    })
                    .response
                    .rect
            });

        ctx.layer_painter(inner.response.layer_id).rect_stroke(
            inner.inner,
            egui::CornerRadius::same(CARD_RADIUS),
            CARD_STROKE,
            egui::StrokeKind::Inside,
        );

        self.card_rect = Some(inner.inner);

        let sprite_rect = self.layout.and_then(|l| l.sprite);
        if let (Some(sprite), Some(rect)) = (&mut self.sprite, sprite_rect)
            && let Some(tex) = sprite.texture(ctx)
        {
            ctx.layer_painter(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::new("nudge_sprite"),
            ))
            .image(
                tex,
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
    }

    fn row_field(ui: &mut egui::Ui, value: &mut String, hint: &str, key: &str) -> egui::Response {
        const ROW_HEIGHT: f32 = 40.0;
        let field_id = egui::Id::new(key);
        let width = ui.available_width();

        let background = ui.painter().add(egui::Shape::Noop);
        let row_top = ui.cursor().top();

        let (row_rect, _) =
            ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::hover());
        let is_focused = ui.ctx().memory(|m| m.has_focus(field_id));
        if is_focused {
            word_jump::intercept_ctrl_word_keys(ui.ctx(), field_id, value);
        }

        let response = ui.put(
            row_rect,
            egui::TextEdit::singleline(value)
                .id(field_id)
                .hint_text(egui::RichText::new(hint).size(16.0))
                // egui 0.34: a custom `.frame(..)` is used verbatim, so its
                // inner_margin — NOT `.margin(..)` — is what insets the text.
                // `.margin()` alone (as in 0.31's `.frame(false)`) leaves the
                // text jammed in the field's top-left corner. Put the 20/12
                // inset on the frame; keep `.margin()` matching so the galley's
                // available width agrees with the frame's content area.
                .frame(egui::Frame::NONE.inner_margin(egui::Margin::symmetric(20, 12)))
                .margin(egui::Margin::symmetric(20, 12))
                .font(egui::FontId::proportional(16.0))
                .text_color(egui::Color32::from_rgb(235, 235, 240)),
        );

        if is_focused {
            let row = egui::Rect::from_x_y_ranges(row_rect.x_range(), row_top..=ui.cursor().top());
            ui.painter().set(
                background,
                egui::epaint::RectShape::filled(
                    row.shrink2(FOCUS_FIELD_INSET),
                    egui::CornerRadius::same(FOCUS_FIELD_RADIUS),
                    egui::Color32::from_white_alpha(20),
                ),
            );
        }

        response
    }

    fn divider(ui: &mut egui::Ui) {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, 1.0), egui::Sense::hover());
        ui.painter().hline(
            rect.x_range(),
            rect.center().y,
            egui::Stroke::new(1.0_f32, egui::Color32::from_white_alpha(50)),
        );
    }

    fn draw_form(&mut self, ui: &mut egui::Ui) {
        ui.style_mut().spacing.item_spacing = egui::vec2(0.0, 0.0);

        let doing_response = Self::row_field(ui, &mut self.doing, "Что я делаю?", "row_doing");
        if self.focus_first {
            doing_response.request_focus();
            self.focus_first = false;
        }

        Self::divider(ui);
        Self::row_field(ui, &mut self.bullshit, "Хуйня?", "row_bullshit");
        Self::divider(ui);
        Self::row_field(
            ui,
            &mut self.next_minutes,
            "Следующий через (мин)",
            "row_minutes",
        );

        if let Some(err) = &self.error_message {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.add_space(20.0);
                ui.label(
                    egui::RichText::new(err)
                        .size(12.0)
                        .color(egui::Color32::from_rgb(255, 80, 80)),
                );
            });
            ui.add_space(12.0);
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn pill_frame(hovered: bool) -> egui::Frame {
        let fill = if hovered {
            egui::Color32::from_rgba_unmultiplied(40, 40, 44, 240)
        } else {
            egui::Color32::from_rgba_unmultiplied(18, 18, 20, 230)
        };
        egui::Frame::NONE
            .fill(fill)
            .corner_radius(egui::CornerRadius::same(14))
            .stroke(egui::Stroke::new(
                1.0_f32,
                egui::Color32::from_white_alpha(12),
            ))
            .shadow(egui::epaint::Shadow {
                offset: [0, 4],
                blur: 12,
                spread: 0,
                color: egui::Color32::from_black_alpha(120),
            })
            .inner_margin(egui::Margin::symmetric(14, 8))
    }

    #[cfg(target_arch = "wasm32")]
    fn draw_pill(&mut self, ctx: &egui::Context) {
        let remaining = self.timer.remaining();
        let mins = remaining.as_secs() / 60;
        let secs = remaining.as_secs() % 60;
        let label = format!("{}:{:02}", mins, secs);

        let is_hovered = match (self.pill_rect, ctx.input(|i| i.pointer.latest_pos())) {
            (Some(rect), Some(pos)) => rect.contains(pos),
            _ => false,
        };

        let inner = egui::Area::new(egui::Id::new("nudge_pill"))
            .anchor(egui::Align2::RIGHT_BOTTOM, egui::vec2(-16.0, -16.0))
            .show(ctx, |ui| {
                let frame_response = Self::pill_frame(is_hovered).show(ui, |ui| {
                    ui.label(
                        egui::RichText::new(label)
                            .size(14.0)
                            .color(egui::Color32::from_gray(210)),
                    );
                });
                let rect = frame_response.response.rect;
                let click = frame_response.response.interact(egui::Sense::click());
                (rect, click)
            });

        let (rect, click_response) = inner.inner;
        self.pill_rect = Some(rect);

        if click_response.clicked() {
            self.show_popup(ctx, TriggerSource::Manual);
        }
    }
}

impl eframe::App for NudgeApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        CLEAR_COLOR_TRANSPARENT
    }

    /// Everything that has to keep running while the popup is hidden.
    ///
    /// eframe 0.34 calls `logic()` on every tick but `ui()` only while the
    /// window is visible (`native/epi_integration.rs`). A hidden popup is the
    /// normal state of this app — it is hidden for the whole interval — so
    /// anything that has to notice the interval elapsing belongs here.
    /// Drawing, and the state transitions that depend on what was drawn,
    /// stay in `ui()`.
    ///
    /// A reveal (macOS: the timer, the status item, the hotkey) is ordered in
    /// here, because `ui()` does not run until the window is visible: placed
    /// first, then shown and given the keyboard. `ui()` makes the
    /// `show_popup` transition on the next frame.
    #[cfg(not(target_arch = "wasm32"))]
    fn logic(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        let revealing = crate::platform::tick(ctx);
        self.apply_pending_sprite();
        if revealing {
            self.restore_window(ctx, frame);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            crate::platform::focus_popup(ctx);
        } else {
            #[cfg(target_os = "macos")]
            self.order_out_if_owed(ctx);
            self.place_window(ctx, frame);
        }
    }

    fn ui(&mut self, root_ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        let ctx_owned = root_ui.ctx().clone();
        let ctx = &ctx_owned;

        if self.popup_visible {
            self.retired_textures.clear();
        }

        if self.first_frame {
            #[cfg(not(target_arch = "wasm32"))]
            crate::platform::adopt_window(frame);

            #[cfg(not(target_arch = "wasm32"))]
            self.enter_waiting(ctx, Some(nudge_state::launch_interval(&self.next_minutes)));
            #[cfg(target_arch = "wasm32")]
            self.show_popup(ctx, TriggerSource::Timer);

            self.first_frame = false;
        }

        #[cfg(not(target_arch = "wasm32"))]
        {
            if crate::platform::take_tray_clicked() && !self.popup_visible {
                self.show_popup(ctx, TriggerSource::Manual);
            }

            let deadline =
                crate::platform::take_timer_fired() | std::mem::take(&mut self.deadline_delivered);
            if deadline && !self.popup_visible {
                self.show_popup(ctx, TriggerSource::Timer);
            }
        }

        #[cfg(target_arch = "wasm32")]
        if !self.popup_visible && self.timer.is_expired() {
            self.show_popup(ctx, TriggerSource::Timer);
        }

        #[cfg(target_arch = "wasm32")]
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(d) = nudge_state::repaint_interval(self.popup_visible) {
            ctx.request_repaint_after(d);
        }

        if self.popup_visible {
            let enter_pressed = ctx.input(|i| i.key_pressed(egui::Key::Enter));
            let esc_pressed = ctx.input(|i| i.key_pressed(egui::Key::Escape));

            if enter_pressed {
                self.hide_popup(ctx, Action::Submit);
                return;
            }
            if esc_pressed {
                self.hide_popup(ctx, Action::Dismiss);
                return;
            }

            if let Some(rect) = self.card_rect {
                let clicked_outside = ctx.input(|i| {
                    i.pointer.any_click()
                        && i.pointer.interact_pos().is_some_and(|p| !rect.contains(p))
                });
                if clicked_outside {
                    self.hide_popup(ctx, Action::SwitchAway);
                    return;
                }
            }

            let focused_now = self.current_foreground_matches(ctx);
            self.latch_arming(focused_now);
            self.retry_focus_until_armed(ctx);
            if self.armed && !focused_now {
                self.hide_popup(ctx, Action::SwitchAway);
                return;
            }
        }

        if self.popup_visible {
            self.place_window(ctx, frame);
            self.draw_card(ctx);
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(card) = self.card_rect {
                crate::platform::sync_card_backdrop(ctx, frame, card);
            }
        } else {
            #[cfg(target_arch = "wasm32")]
            self.draw_pill(ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Rect, pos2, vec2};

    #[test]
    fn clear_color_is_fully_transparent() {
        assert_eq!(CLEAR_COLOR_TRANSPARENT, [0.0, 0.0, 0.0, 0.0]);
    }

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), vec2(w, h))
    }

    fn placement() -> Placement {
        Placement {
            geometry: ScreenGeometry {
                monitor: rect(0.0, 0.0, 1470.0, 956.0),
                work: rect(0.0, 33.0, 1470.0, 833.0),
            },
            ppp: 2.0,
            sprite: Some(([720, 430], (-41, -277), 679)),
        }
    }

    #[test]
    fn a_pending_sprite_is_applied_only_while_hidden() {
        let cfg = SpriteConfig {
            enabled: true,
            path: "/tmp/sprite.png".to_string(),
            x: -41,
            y: -277,
            card_width: 679,
        };
        assert_eq!(sprite_apply(Some(&cfg), true), None, "visible: it waits");
        assert_eq!(
            sprite_apply(Some(&cfg), false),
            Some(&cfg),
            "hidden: applied"
        );
        assert_eq!(sprite_apply(None, false), None, "nothing pending");
    }

    #[test]
    fn a_window_never_placed_is_stale() {
        assert!(placement_is_stale(None, &placement()));
    }

    #[test]
    fn the_same_placement_is_not_stale() {
        assert!(!placement_is_stale(Some(&placement()), &placement()));
    }

    #[test]
    fn any_changed_input_makes_the_placement_stale() {
        let base = placement();
        let mut cases = Vec::new();
        let mut c = base;
        c.geometry.work = rect(0.0, 33.0, 1470.0, 760.0);
        cases.push(("work area", c));
        let mut c = base;
        c.geometry.monitor = rect(1470.0, 0.0, 1920.0, 1080.0);
        cases.push(("monitor", c));
        let mut c = base;
        c.ppp = 1.0;
        cases.push(("scale", c));
        let mut c = base;
        c.sprite = Some(([721, 430], (-41, -277), 679));
        cases.push(("sprite pixels", c));
        let mut c = base;
        c.sprite = Some(([720, 430], (-40, -277), 679));
        cases.push(("sprite x", c));
        let mut c = base;
        c.sprite = Some(([720, 430], (-41, -276), 679));
        cases.push(("sprite y", c));
        let mut c = base;
        c.sprite = Some(([720, 430], (-41, -277), 680));
        cases.push(("card width", c));
        let mut c = base;
        c.sprite = None;
        cases.push(("sprite off", c));
        for (what, now) in cases {
            assert!(placement_is_stale(Some(&base), &now), "{what}");
        }
    }
}
