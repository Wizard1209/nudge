use eframe::egui;

use crate::nudge_state::{self, Action, TriggerSource};
use crate::timer::Timer;
use crate::word_jump;

/// Framebuffer clear color for the transparent spotlight window. Using an
/// opaque value (even dark grey) breaks native transparency — Windows DWM
/// composites the alpha channel straight from the framebuffer, so anything
/// non-zero produces the grey box halo seen in early builds.
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

/// Card corner radius, in points. Shared by the card frame, the contour and
/// the focused-row background so the three curves are concentric by
/// construction — an inset highlight with its own smaller radius reads as a
/// second, misaligned outline where the two corners diverge.
pub(crate) const CARD_RADIUS: u8 = 16;

/// Opacity of the card's fill. Two values, because the fill has two different
/// jobs depending on whether the platform can put a frosted backdrop behind
/// the window (spec §2).
///
/// With one — `platform::sync_card_backdrop` on macOS — the desktop reaching
/// the card is already blurred, and the fill's job is only to tint it; at the
/// near-opaque value the frost was measurably present and visually absent —
/// over a bright checkerboard the card read one luminance level above what a
/// fully opaque card would. Without a backdrop the fill *is* the surface, and
/// the low value would leave whatever is behind the window legible straight
/// through the card.
///
/// This is the one place the card's appearance depends on the OS, so it is a
/// `cfg` rather than a question routed through the platform seam: the seam
/// dispatches behaviour, and there is no behaviour here to dispatch. The
/// browser build takes the no-backdrop value — it is the same egui code, and
/// nothing draws a CSS filter behind its canvas.
#[cfg(target_os = "macos")]
const CARD_FILL_ALPHA: u8 = 120;
#[cfg(not(target_os = "macos"))]
const CARD_FILL_ALPHA: u8 = 220;

/// The card's shadow, and which layer draws it.
///
/// macOS draws its own. Once egui paints none, the only thing in the window's
/// alpha mask is the card, so AppKit's window shadow traces the card itself —
/// the platform's shadow, with the falloff every other floating panel has.
/// Keeping both is what laid a hard rim just outside the contour: AppKit
/// shadows the alpha silhouette, and with egui's soft shadow in that alpha the
/// silhouette is larger than the card and its edge lands away from the
/// contour. One layer or the other, never both — see
/// `card_casts_one_soft_shadow_and_no_hard_rim`.
///
/// Elsewhere egui draws it, because nothing else will.
#[cfg(target_os = "macos")]
const CARD_SHADOW: egui::epaint::Shadow = egui::epaint::Shadow::NONE;
#[cfg(not(target_os = "macos"))]
const CARD_SHADOW: egui::epaint::Shadow = egui::epaint::Shadow {
    offset: [0, 12],
    blur: 40,
    spread: 0,
    // `Color32::from_black_alpha` is not const; this is its expansion.
    color: egui::Color32::from_rgba_premultiplied(0, 0, 0, 160),
};

/// The card's contour. Separation from the desktop cannot come from the
/// shadow alone: the shadow is black, so over a dark background behind the
/// window it contributes nothing and the card loses its boundary entirely.
/// A light hairline is the only cue that survives both a bright and a dark
/// backdrop, so it is load-bearing rather than decorative — see the
/// `card_contour_*` assertions in `tests/macos_native.rs`.
const CARD_STROKE: egui::Stroke = egui::Stroke {
    width: 1.0,
    // `Color32::from_white_alpha` is not const; this is its expansion —
    // white premultiplied by the same alpha.
    color: egui::Color32::from_rgba_premultiplied(60, 60, 60, 60),
};

pub struct NudgeApp {
    // Form fields
    doing: String,
    bullshit: String,
    next_minutes: String,
    focus_first: bool,
    center_once: bool,

    // Shared state
    timer: Timer,
    trigger_source: TriggerSource,
    popup_visible: bool,
    error_message: Option<String>,
    /// Last known card bounding rect, used to detect clicks outside the card.
    /// Populated by `draw_card` once per frame.
    card_rect: Option<egui::Rect>,
    #[cfg(target_arch = "wasm32")]
    pill_rect: Option<egui::Rect>,
    /// Did this open of the popup ever actually get the keyboard? While true,
    /// the per-frame focus-loss check is active. While false (e.g. a
    /// fullscreen game refused to release foreground), focus-loss never closes
    /// the popup — only Enter / Esc / click-outside can. See spec §4.
    ///
    /// Windows samples this once, in `show_popup`, because `GetForegroundWindow`
    /// answers truthfully the instant `SetForegroundWindow` returns. macOS
    /// cannot: `activateIgnoringOtherApps` is a request to the window server
    /// that no same-call read reflects, so there it is a latch that closes on
    /// the first frame the window is key. Both are "did this open get focus",
    /// they just observe it at the only moment their OS can answer.
    armed: bool,

    // Native window handle for Win32 API calls.
    // The tray icon itself lives on a dedicated thread (see tray_bridge);
    // NudgeApp only signals popup state changes to it.
    #[cfg(target_os = "windows")]
    hwnd: Option<isize>,
}

impl NudgeApp {
    pub fn new(cc: &eframe::CreationContext<'_>, default_minutes: f64) -> Self {
        let mut visuals = egui::Visuals::dark();
        // Transparent panel background so the card sits on top of wallpaper/desktop
        visuals.panel_fill = egui::Color32::TRANSPARENT;
        visuals.window_fill = egui::Color32::TRANSPARENT;
        visuals.widgets.noninteractive.fg_stroke.color = HINT_TEXT_COLOR;
        visuals.widgets.inactive.fg_stroke.color = HINT_TEXT_COLOR;
        // The only place a hint color survives in egui 0.34 — TextEdit
        // clobbers per-hint RichText colors with this (see HINT_TEXT_COLOR).
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
        // loop never paints on its own: the window sits as a transparent
        // never-rendered overlay, `ui()` never runs, so hwnd is never handed
        // to the tray (its Show/hotkey wakeups need it) and the frozen
        // first-launch timer can never be started by closing the popup.
        // Observed live on Windows: launch unfocused → no frame until the
        // first real input event hits the window.
        cc.egui_ctx.request_repaint();

        Self {
            doing: String::new(),
            bullshit: String::new(),
            next_minutes: default_minutes.to_string(),
            focus_first: true,
            center_once: true,
            // Spec §4: timer is frozen until the user closes the popup for
            // the first time (Submit / Esc / Switch). hide_popup → reset()
            // unfreezes it with the interval the user chose.
            timer: Timer::frozen(),
            trigger_source: TriggerSource::Timer,
            // Born hidden. The launch popup is NOT a special constructor
            // state: the first frame performs a regular show_popup
            // transition (see `ui`), so "visible" has exactly one entry
            // point and the show side effects (foreground grab, arming)
            // can't be skipped by construction.
            popup_visible: false,
            error_message: None,
            card_rect: None,
            #[cfg(target_arch = "wasm32")]
            pill_rect: None,
            armed: false,
            #[cfg(target_os = "windows")]
            hwnd: None,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn now_timestamp() -> String {
        chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false)
    }

    #[cfg(target_arch = "wasm32")]
    fn now_timestamp() -> String {
        let date = js_sys::Date::new_0();
        let offset_total = -(date.get_timezone_offset() as i32); // minutes east of UTC
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

        // Park the countdown: with the popup on screen the timer shouldn't
        // keep ticking down (nor the tray icon animate). Restarts on
        // hide_popup.
        #[cfg(not(target_arch = "wasm32"))]
        crate::platform::set_popup_visible(true);

        // Take the keyboard. The window being visible is not enough on
        // either platform — Win32 refuses SetForegroundWindow to a
        // background process, and macOS hands out no key window without an
        // AppKit activation — and a popup without focus is one the user
        // cannot answer or dismiss.
        #[cfg(not(target_arch = "wasm32"))]
        crate::platform::focus_popup(ctx);

        // Spec §4: focus-loss closes only if the popup actually got focus at
        // open-time. This is the first sample. On Windows and WASM it is also
        // the last one — the answer is already true here. On macOS it is
        // almost certainly false and the latch in `ui` is what arms us; see
        // `current_foreground_matches`.
        self.armed = self.current_foreground_matches(ctx);
    }

    /// Source of truth for "is our popup the OS-foreground / page-focused
    /// surface right now?" — used both at open-time to set `armed` and in
    /// the per-frame loop to detect focus loss.
    ///
    /// Windows and WASM go through `GetForegroundWindow` / `document.hasFocus()`
    /// directly rather than eframe's `viewport().focused`, because there that
    /// field is filled from a window event and so lags a frame behind
    /// `WM_ACTIVATE`.
    ///
    /// **macOS reads `viewport().focused`, and the lag objection above does not
    /// carry over.** egui-winit 0.34 refreshes that field from
    /// `Window::has_focus()` in `update_viewport_info`, called at the top of
    /// every frame immediately before `take_egui_input` (`egui-winit/src/lib.rs`
    /// :1278, `eframe/src/native/glow_integration.rs`:557). On macOS
    /// `Window::has_focus()` is `NSWindow::isKeyWindow` — a live AppKit read,
    /// not a cached event — so this is exactly as current as an
    /// `NSApplication::isActive` call made from here, and it answers the
    /// sharper question: whether *this window* is the one Esc and Enter will
    /// reach, which is the condition spec §4 is actually about. It also needs
    /// no `MainThreadMarker`, so the arming logic stays reachable from the
    /// headless kittest tier.
    fn current_foreground_matches(&self, ctx: &egui::Context) -> bool {
        // Only the macOS arm reads it; the others ask their OS directly.
        let _ = ctx;

        #[cfg(target_os = "windows")]
        {
            use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
            let Some(hwnd_val) = self.hwnd else {
                return false;
            };
            let fg = unsafe { GetForegroundWindow() };
            !fg.0.is_null() && fg.0 as isize == hwnd_val
        }
        #[cfg(target_arch = "wasm32")]
        {
            return web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.has_focus().ok())
                .unwrap_or(false);
        }
        #[cfg(target_os = "macos")]
        {
            // `None` = winit has not reported on the window yet (first frames,
            // and every kittest harness). Not focused is the safe reading:
            // it delays arming, it cannot cause a close.
            ctx.input(|i| i.viewport().focused) == Some(true)
        }
        #[cfg(not(any(target_os = "windows", target_os = "macos", target_arch = "wasm32")))]
        {
            // The fallback host has no OS integration at all (see
            // `platform::fallback`): it never nudges, nothing activates the
            // window, and a focus-loss close would be the only spec §4
            // behaviour present on a platform that has none of the rest.
            false
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
        // Named unconditionally so the parameter is not "unused" on the
        // platforms whose arm below is compiled out.
        let _ = focused_now;

        #[cfg(target_os = "macos")]
        if focused_now {
            self.armed = true;
        }
    }

    fn hide_popup(&mut self, ctx: &egui::Context, action: Action) {
        // The entire spec §4 close decision lives in `decide_close`; this
        // method is its executor. The only thing we resolve here that the
        // decision can't is the journal-write I/O, because that can fail at
        // the OS / browser layer (and a failure must keep the popup open).
        let outcome = match nudge_state::decide_close(
            self.trigger_source,
            action,
            &self.doing,
            &self.bullshit,
            &self.next_minutes,
        ) {
            Ok(o) => o,
            Err(e) => {
                // Explicit Submit with an invalid interval: surface and keep
                // the popup open (no outcome to execute).
                self.error_message = Some(e.to_string());
                return;
            }
        };

        if outcome.write_journal {
            // The interval the journal records is whatever the decision is
            // about to reset the timer to. `write_journal` implies Submit,
            // and Submit always resets, so `reset_timer` is `Some` here.
            let minutes: f64 = outcome
                .reset_timer
                .map(|d| d.as_secs_f64() / 60.0)
                .unwrap_or(10.0);
            let trigger = match self.trigger_source {
                TriggerSource::Timer => "timer",
                TriggerSource::Manual => "manual",
            };
            let event = crate::journal::new_submitted_event(
                Self::now_timestamp(),
                trigger,
                self.doing.clone(),
                self.bullshit.clone(),
                minutes,
            );

            #[cfg(not(target_arch = "wasm32"))]
            {
                let path = crate::journal::resolve_default_journal_path();
                if let Err(e) = crate::journal::write_event(&path, &event) {
                    self.error_message = Some(e.to_string());
                    return; // keep popup open, don't reset timer
                }
            }

            #[cfg(target_arch = "wasm32")]
            {
                if let Err(e) = crate::journal::write_event_to_localstorage(&event) {
                    self.error_message = Some(e.to_string());
                    return;
                }
            }
        }

        // Clear error on successful action
        self.error_message = None;

        // Shared: timer reset. `Some(interval)` ⇒ reset; `None` ⇒ leave the
        // live timer ticking (manual Esc / Switch).
        if let Some(interval) = outcome.reset_timer {
            self.timer.reset(interval);
        }

        // Shared: reset form per the decision. Only Submit clears
        // doing/bullshit; Esc and Switch preserve them so the next open
        // resumes where the user left off.
        if outcome.clear_form {
            self.doing.clear();
            self.bullshit.clear();
        }
        self.focus_first = true;
        self.popup_visible = false;
        self.armed = false;
        // Drop the cached card bounds. Without this, a stale rect from the
        // previous show survives across hide/show — and the next click that
        // reopens the popup (e.g. a pill click) is then mis-detected as
        // "outside the card" on the very next frame, instantly switching us
        // back away.
        self.card_rect = None;

        // Tell egui the viewport is hidden. Without this, egui's event
        // loop keeps pumping/scheduling for the "visible" viewport even
        // when we SW_HIDE the HWND directly — winit then burns a full
        // core on the main thread despite update() never being called.
        #[cfg(not(target_arch = "wasm32"))]
        ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));

        // Windows-only: hide the HWND directly as well. The viewport command
        // above is what stops winit burning a core; this is what the tray
        // thread's ShowWindow(SW_RESTORE) later undoes.
        #[cfg(target_os = "windows")]
        if let Some(hwnd_val) = self.hwnd {
            unsafe {
                use windows::Win32::Foundation::HWND;
                use windows::Win32::UI::WindowsAndMessaging::*;
                let _ = ShowWindow(HWND(hwnd_val as *mut _), SW_HIDE);
            }
        }

        // Native: hand the platform the new deadline, then tell it the popup
        // is gone. Whatever owns the countdown there — the tray thread on
        // Windows, `App::logic` on macOS — takes it from here and wakes the
        // popup when it expires.
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(interval) = outcome.reset_timer {
                let deadline = std::time::Instant::now() + interval;
                crate::platform::set_timer_state(deadline, interval);
            }
            crate::platform::set_popup_visible(false);
        }
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
            // No `.stroke()` here on purpose: a Frame paints its stroke *before*
            // its content, so a row background (spec §3) would paint over it.
            // `draw_card` paints the contour after the Area instead.
            .shadow(CARD_SHADOW)
            // Zero inner margin so rows + dividers span full card width;
            // row content gets its own horizontal/vertical padding via TextEdit::margin.
            .inner_margin(egui::Margin::ZERO)
    }

    fn draw_card(&mut self, ctx: &egui::Context) {
        // egui 0.34 split screen_rect() into viewport_rect()/content_rect();
        // content_rect() is the in-window content area we size the card against.
        let screen = ctx.content_rect();
        let card_width = 480.0_f32.min(screen.width() - 48.0);
        let top_offset = screen.height() * 0.25;

        let inner = egui::Area::new(egui::Id::new("nudge_card"))
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, top_offset))
            // Without this the Area itself registers a focusable click widget
            // (Sense::click() implies FOCUSABLE) and shows up as a phantom
            // 4th Tab stop between row_minutes and row_doing.
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

        // Contour last: it has to survive the full-bleed row background that
        // `row_field` paints over the card's own edge.
        ctx.layer_painter(inner.response.layer_id).rect_stroke(
            inner.inner,
            egui::CornerRadius::same(CARD_RADIUS),
            CARD_STROKE,
            egui::StrokeKind::Inside,
        );

        self.card_rect = Some(inner.inner);
    }

    /// `corners` is the rounding the focused-row background takes: the card's
    /// radius on the corners this row shares with the card, square elsewhere.
    fn row_field(
        ui: &mut egui::Ui,
        value: &mut String,
        hint: &str,
        key: &str,
        corners: egui::CornerRadius,
    ) -> egui::Response {
        const ROW_HEIGHT: f32 = 40.0;
        let field_id = egui::Id::new(key);
        let width = ui.available_width();

        // Reserve the background's slot before laying the field out. It has to
        // be painted *under* the text, but its rect is not known until after —
        // see below for why the row is not the height it asks for.
        let background = ui.painter().add(egui::Shape::Noop);
        let row_top = ui.cursor().top();

        let (row_rect, _) =
            ui.allocate_exact_size(egui::vec2(width, ROW_HEIGHT), egui::Sense::hover());
        let is_focused = ui.ctx().memory(|m| m.has_focus(field_id));
        if is_focused {
            // Replace egui's ASCII-only word jump with a Unicode-aware one
            // so Ctrl+Arrow / Ctrl+Backspace / Ctrl+Delete behave correctly
            // on Cyrillic text. Must run BEFORE TextEdit drains the events.
            word_jump::intercept_ctrl_word_keys(ui.ctx(), field_id, value);

            // Spec §3: the focused row is marked by a change of *row
            // background* — not by a plate with edges of its own. So the tint
            // spans the row edge to edge and takes the card's radius on the
            // corners it touches; a shrunk rect with a smaller radius cannot
            // stay concentric with the card and shows up as a second outline.
            //
            // egui 0.34 corrected its alpha compositing (0.31 over-brightened
            // low alphas — `2` there rendered like a +20 brightness jump); on
            // 0.34 the same value is nearly invisible, so alpha 20 is what
            // lands a clearly-visible but still-subtle highlight.
        }

        let response = ui.put(
            row_rect,
            egui::TextEdit::singleline(value)
                .id(field_id)
                // No .color() on the hint: egui 0.34 overwrites it with
                // visuals.weak_text_color(), which new() sets to
                // HINT_TEXT_COLOR. Only the size survives here.
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
            // The row is taller than `ROW_HEIGHT`: the TextEdit's own height is
            // its 16pt line plus 12pt of margin top and bottom, and `put`
            // advances the cursor by what the widget actually took, not by what
            // was allocated for it. The divider therefore sits ~3pt below
            // `row_rect`'s bottom edge, and a tint painted on `row_rect` stops
            // short of it, leaving a strip of untinted card between the
            // highlight and the line — the highlight reads as misaligned with
            // its own row. So the tint is painted on the band the row really
            // consumed, measured from the cursor rather than declared.
            let row = egui::Rect::from_x_y_ranges(row_rect.x_range(), row_top..=ui.cursor().top());
            ui.painter().set(
                background,
                egui::epaint::RectShape::filled(row, corners, egui::Color32::from_white_alpha(20)),
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
        // Reset spacing so dividers sit flush against rows
        ui.style_mut().spacing.item_spacing = egui::vec2(0.0, 0.0);

        let top = egui::CornerRadius {
            nw: CARD_RADIUS,
            ne: CARD_RADIUS,
            sw: 0,
            se: 0,
        };
        let bottom = egui::CornerRadius {
            nw: 0,
            ne: 0,
            sw: CARD_RADIUS,
            se: CARD_RADIUS,
        };

        let doing_response = Self::row_field(ui, &mut self.doing, "Что я делаю?", "row_doing", top);
        if self.focus_first {
            doing_response.request_focus();
            self.focus_first = false;
        }

        Self::divider(ui);
        Self::row_field(
            ui,
            &mut self.bullshit,
            "Хуйня?",
            "row_bullshit",
            egui::CornerRadius::ZERO,
        );
        Self::divider(ui);
        Self::row_field(
            ui,
            &mut self.next_minutes,
            "Следующий через (мин)",
            "row_minutes",
            bottom,
        );

        // Show error message if journal write failed
        if let Some(err) = &self.error_message {
            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(err)
                    .size(12.0)
                    .color(egui::Color32::from_rgb(255, 80, 80)),
            );
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
            .stroke(egui::Stroke::new(1.0, egui::Color32::from_white_alpha(12)))
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

        // Check hover based on previous frame's rect; first frame has no rect → no hover.
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
    #[cfg(not(target_arch = "wasm32"))]
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        crate::platform::tick(ctx);
    }

    // `frame` only feeds the card backdrop, which has no web counterpart.
    #[cfg_attr(target_arch = "wasm32", expect(unused_variables))]
    fn ui(&mut self, root_ui: &mut egui::Ui, frame: &mut eframe::Frame) {
        // eframe 0.34 replaced `App::update(ctx)` with `App::ui(ui)`. The
        // `ui` we're handed is the central panel (transparent, no margin); we
        // drive everything off its context exactly as the old `update` did.
        let ctx_owned = root_ui.ctx().clone();
        let ctx = &ctx_owned;

        // === First frame setup ===
        if self.center_once {
            // Position is set at launch from primary monitor dimensions (see main.rs).
            // Here we only capture the native window handle and store it for the tray.

            // Extract native window handle and share with tray event handlers
            #[cfg(target_os = "windows")]
            {
                use raw_window_handle::HasWindowHandle;
                if let Ok(wh) = frame.window_handle()
                    && let raw_window_handle::RawWindowHandle::Win32(h) = wh.as_raw()
                {
                    let hwnd_val = h.hwnd.get();
                    self.hwnd = Some(hwnd_val);
                    crate::platform::note_window_handle(hwnd_val);

                    // Exclude window from ALT+TAB by applying WS_EX_TOOLWINDOW
                    // (also hides from taskbar as a side effect; with_taskbar(false)
                    // already handles the taskbar but tool-window style is what
                    // reliably hides from ALT+TAB).
                    unsafe {
                        use windows::Win32::Foundation::HWND;
                        use windows::Win32::UI::WindowsAndMessaging::{
                            GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_TOOLWINDOW,
                        };
                        let h = HWND(hwnd_val as *mut _);
                        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
                        let _ =
                            SetWindowLongPtrW(h, GWL_EXSTYLE, ex | (WS_EX_TOOLWINDOW.0 as isize));
                    }
                }
            }

            // Spec §1/§4: present the launch popup. The app is constructed
            // hidden, so this is the one and only entry into the visible
            // state — the same show_popup transition as timer/tray/hotkey
            // opens, with the same side effects (force_foreground so the
            // popup gets keyboard focus even when the launcher — autostart
            // at logon — didn't hand us foreground, then arm the
            // focus-loss check). Runs after hwnd capture above because
            // force_foreground needs the handle.
            self.show_popup(ctx, TriggerSource::Timer);

            self.center_once = false;
        }

        // === Native-only: check platform event flags ===
        // Whoever noticed the event — the Windows tray thread, or `logic()`
        // on macOS — has already made the window visible and raised a flag.
        // The state transition itself happens here, once, for both.
        #[cfg(not(target_arch = "wasm32"))]
        {
            if crate::platform::take_tray_clicked() && !self.popup_visible {
                self.show_popup(ctx, TriggerSource::Manual);
            }

            if crate::platform::take_timer_fired() && !self.popup_visible {
                self.show_popup(ctx, TriggerSource::Timer);
            }
        }

        // === WASM-only: timer check (update() always runs on WASM) ===
        #[cfg(target_arch = "wasm32")]
        if !self.popup_visible && self.timer.is_expired() {
            self.show_popup(ctx, TriggerSource::Timer);
        }

        // === Periodic repaint ===
        // WASM: always repaint — update() polls timer expiry here (no
        // background thread to ShowWindow us awake).
        // Native: only while popup is visible. When hidden, tray clicks
        // and the timer-expiry thread wake the event loop via ShowWindow,
        // so a standing 1 Hz wakeup would just burn CPU compositing the
        // transparent layered window through DWM.
        #[cfg(target_arch = "wasm32")]
        ctx.request_repaint_after(std::time::Duration::from_secs(1));
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(d) = nudge_state::repaint_interval(self.popup_visible) {
            ctx.request_repaint_after(d);
        }

        // === Shared: keyboard handling ===
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

            // Click outside card → switch away (spec §4): hide popup, reset
            // timer, but KEEP doing/bullshit so the next open resumes here.
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

            // Window focus-loss → switch away (spec §4).
            // `armed` means this open of the popup did get the keyboard at
            // some point — sampled in show_popup on Windows / WASM, latched
            // here on macOS (see `latch_arming`). If it never did, focus-loss
            // never fires and Esc / Enter / click-outside remain the only ways
            // out.
            let focused_now = self.current_foreground_matches(ctx);
            self.latch_arming(focused_now);
            if self.armed && !focused_now {
                self.hide_popup(ctx, Action::SwitchAway);
                return;
            }
        }

        // === Shared: render UI ===
        // `root_ui` (handed to us by eframe) is the transparent, margin-less
        // central panel — wallpaper/desktop shows through around the card.
        // We paint only the floating card Area on top of it.
        if self.popup_visible {
            self.draw_card(ctx);
            // The blur behind the card is an OS-side panel, so it has to be
            // told where the card ended up — every frame, because the card's
            // width tracks the screen and its height tracks its content.
            #[cfg(not(target_arch = "wasm32"))]
            if let Some(card) = self.card_rect {
                crate::platform::sync_card_backdrop(ctx, frame, card);
            }
        } else {
            // WASM: show pill with countdown at bottom-right
            // Native: window is SW_HIDE'd, nothing to render
            #[cfg(target_arch = "wasm32")]
            self.draw_pill(ctx);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clear_color_is_fully_transparent() {
        assert_eq!(CLEAR_COLOR_TRANSPARENT, [0.0, 0.0, 0.0, 0.0]);
    }
}
