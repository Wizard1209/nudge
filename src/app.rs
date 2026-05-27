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
/// like "Что я делаю?" are comfortably legible. Set both on the egui visuals
/// (so the widget defaults match) and on each field's hint RichText.
const HINT_TEXT_COLOR: egui::Color32 = egui::Color32::from_gray(170);

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
    /// Snapshot taken in `show_popup`: did the OS hand us foreground at the
    /// moment the popup opened? When true, per-frame focus-loss check is
    /// active. When false (e.g. a fullscreen game refused to release
    /// foreground), focus-loss never closes the popup — only Enter / Esc /
    /// click-outside can. See spec §4.
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
        cc.egui_ctx.set_visuals(visuals);

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
            popup_visible: true,
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
        chrono::Local::now()
            .to_rfc3339_opts(chrono::SecondsFormat::Millis, false)
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

        // Park the icon animator: with the popup on screen the icon
        // shouldn't keep ticking down. It'll restart on hide_popup.
        #[cfg(target_os = "windows")]
        crate::tray_bridge::set_popup_visible(true);

        #[cfg(target_os = "windows")]
        if let Some(hwnd_val) = self.hwnd {
            use windows::Win32::Foundation::HWND;
            crate::tray_bridge::force_foreground(HWND(hwnd_val as *mut _));
        }

        // Spec §4: focus-loss closes only if the popup actually got focus at
        // open-time. Snapshot the OS's foreground state synchronously now,
        // right after force_foreground (native) or as soon as the popup
        // becomes visible (WASM). Stays fixed for the lifetime of this open.
        self.armed = self.current_foreground_matches();
    }

    /// Source of truth for "is our popup the OS-foreground / page-focused
    /// surface right now?" — used both at open-time to set `armed` and in
    /// the per-frame loop to detect focus loss. Goes through Win32 /
    /// `document.hasFocus()` directly, not eframe's `viewport().focused`,
    /// because the latter lags by a frame after WM_ACTIVATE / focus events.
    fn current_foreground_matches(&self) -> bool {
        #[cfg(target_os = "windows")]
        {
            use windows::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
            let Some(hwnd_val) = self.hwnd else { return false };
            let fg = unsafe { GetForegroundWindow() };
            return !fg.0.is_null() && fg.0 as isize == hwnd_val;
        }
        #[cfg(target_arch = "wasm32")]
        {
            return web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.has_focus().ok())
                .unwrap_or(false);
        }
        #[cfg(not(any(target_os = "windows", target_arch = "wasm32")))]
        {
            false
        }
    }

    fn hide_popup(&mut self, ctx: &egui::Context, action: Action) {
        // Parse the interval field. A parse error means the user typed an
        // explicit non-positive number (e.g. "-5" or "0"). On Submit that is
        // a validation error — surface it and keep the popup open. On
        // Dismiss we silently fall back to the 10-minute default so Esc
        // still works even with garbage in the field.
        let interval = match nudge_state::parse_interval(&self.next_minutes) {
            Ok(d) => d,
            Err(e) => {
                if action == Action::Submit {
                    self.error_message = Some(e.to_string());
                    return;
                }
                std::time::Duration::from_secs(10 * 60)
            }
        };
        let minutes: f64 = interval.as_secs_f64() / 60.0;

        // Spec §4: Submit with both text fields empty is the "update timer
        // without journaling" path — skip the write but still reset the
        // timer below.
        if nudge_state::should_write_journal(action, &self.doing, &self.bullshit) {
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

        // Shared: timer reset logic
        let should_reset = nudge_state::should_reset_timer(self.trigger_source, action);
        if should_reset {
            self.timer.reset(interval);
        }

        // Shared: reset form. Per spec §4 only Submit (Enter) clears
        // doing/bullshit; Esc and Switch both preserve them so the next open
        // resumes where the user left off.
        if nudge_state::should_clear_form(action) {
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

        // Native-only: hide window + schedule background timer wakeup
        #[cfg(target_os = "windows")]
        {
            if let Some(hwnd_val) = self.hwnd {
                unsafe {
                    use windows::Win32::Foundation::HWND;
                    use windows::Win32::UI::WindowsAndMessaging::*;
                    let _ = ShowWindow(HWND(hwnd_val as *mut _), SW_HIDE);
                }
            }

            // The tray thread handles both icon animation and the
            // popup-fire wakeup (it ShowWindow's our HWND once the timer
            // expires). Hand it the new deadline so it knows when.
            if should_reset {
                let deadline = std::time::Instant::now() + interval;
                crate::tray_bridge::set_timer_state(deadline, interval);
            }
            crate::tray_bridge::set_popup_visible(false);
        }
    }

    fn card_frame() -> egui::Frame {
        egui::Frame::NONE
            .fill(egui::Color32::from_rgba_unmultiplied(24, 24, 27, 220))
            .corner_radius(egui::CornerRadius::same(16))
            .stroke(egui::Stroke::new(
                1.0,
                egui::Color32::from_white_alpha(16),
            ))
            .shadow(egui::epaint::Shadow {
                offset: [0, 12],
                blur: 40,
                spread: 0,
                color: egui::Color32::from_black_alpha(160),
            })
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
        self.card_rect = Some(inner.inner);
    }

    fn row_field(
        ui: &mut egui::Ui,
        value: &mut String,
        hint: &str,
        key: &str,
    ) -> egui::Response {
        const ROW_HEIGHT: f32 = 40.0;
        let field_id = egui::Id::new(key);
        let width = ui.available_width();
        let (row_rect, _) = ui.allocate_exact_size(
            egui::vec2(width, ROW_HEIGHT),
            egui::Sense::hover(),
        );
        let is_focused = ui.ctx().memory(|m| m.has_focus(field_id));
        if is_focused {
            // Replace egui's ASCII-only word jump with a Unicode-aware one
            // so Ctrl+Arrow / Ctrl+Backspace / Ctrl+Delete behave correctly
            // on Cyrillic text. Must run BEFORE TextEdit drains the events.
            word_jump::intercept_ctrl_word_keys(ui.ctx(), field_id, value);

            // Subtle row tint, inset from the card edge and softly rounded
            // so it visually sits INSIDE the card instead of butting up
            // against the stroke/rounded corners. egui 0.34 corrected its
            // alpha compositing (0.31 over-brightened low alphas — `2` there
            // rendered like a +20 brightness jump); on 0.34 the same value is
            // nearly invisible, so we use alpha 20 to land a clearly-visible
            // but still-subtle highlight. Tuned against design-focus-highlight.
            let inset_rect = row_rect.shrink2(egui::vec2(8.0, 4.0));
            ui.painter().rect_filled(
                inset_rect,
                egui::CornerRadius::same(6),
                egui::Color32::from_white_alpha(20),
            );
        }
        ui.put(
            row_rect,
            egui::TextEdit::singleline(value)
                .id(field_id)
                .hint_text(
                    egui::RichText::new(hint)
                        .size(16.0)
                        .color(HINT_TEXT_COLOR),
                )
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
        )
    }

    fn divider(ui: &mut egui::Ui) {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(width, 1.0),
            egui::Sense::hover(),
        );
        ui.painter().hline(
            rect.x_range(),
            rect.center().y,
            egui::Stroke::new(1.0, egui::Color32::from_white_alpha(50)),
        );
    }

    fn draw_form(&mut self, ui: &mut egui::Ui) {
        // Reset spacing so dividers sit flush against rows
        ui.style_mut().spacing.item_spacing = egui::vec2(0.0, 0.0);

        let doing_response = Self::row_field(ui, &mut self.doing, "Что я делаю?", "row_doing");
        if self.focus_first {
            doing_response.request_focus();
            self.focus_first = false;
        }

        Self::divider(ui);
        Self::row_field(ui, &mut self.bullshit, "Хуйня?", "row_bullshit");
        Self::divider(ui);
        Self::row_field(ui, &mut self.next_minutes, "Следующий через (мин)", "row_minutes");

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
            .stroke(egui::Stroke::new(
                1.0,
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

    fn ui(&mut self, root_ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
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
                if let Ok(wh) = _frame.window_handle() {
                    if let raw_window_handle::RawWindowHandle::Win32(h) = wh.as_raw() {
                        let hwnd_val = h.hwnd.get();
                        self.hwnd = Some(hwnd_val);
                        crate::tray_bridge::store_hwnd(hwnd_val);

                        // Exclude window from ALT+TAB by applying WS_EX_TOOLWINDOW
                        // (also hides from taskbar as a side effect; with_taskbar(false)
                        // already handles the taskbar but tool-window style is what
                        // reliably hides from ALT+TAB).
                        unsafe {
                            use windows::Win32::Foundation::HWND;
                            use windows::Win32::UI::WindowsAndMessaging::{
                                GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE,
                                WS_EX_TOOLWINDOW,
                            };
                            let h = HWND(hwnd_val as *mut _);
                            let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
                            let _ = SetWindowLongPtrW(
                                h,
                                GWL_EXSTYLE,
                                ex | (WS_EX_TOOLWINDOW.0 as isize),
                            );
                        }
                    }
                }
            }

            // Spec §4: initial popup shown by Self::new bypasses show_popup,
            // so arm it here on the first frame — once hwnd is captured
            // (native) and the page is ready (WASM).
            if self.popup_visible {
                self.armed = self.current_foreground_matches();
            }

            self.center_once = false;
        }

        // === Native-only: check tray event flags ===
        // Event handlers run on Windows message thread (set up in main.rs),
        // they restore the window + set flags. We check flags here.
        #[cfg(target_os = "windows")]
        {
            if crate::tray_bridge::take_tray_clicked() {
                if !self.popup_visible {
                    self.show_popup(ctx, TriggerSource::Manual);
                }
            }

            // Background timer thread restored window + set flag
            if crate::tray_bridge::take_timer_fired() {
                if !self.popup_visible {
                    self.show_popup(ctx, TriggerSource::Timer);
                }
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
                        && i.pointer
                            .interact_pos()
                            .map_or(false, |p| !rect.contains(p))
                });
                if clicked_outside {
                    self.hide_popup(ctx, Action::SwitchAway);
                    return;
                }
            }

            // Window focus-loss → switch away (spec §4).
            // `armed` was snapshotted in show_popup: true iff we actually got
            // foreground at open-time. If we didn't, focus-loss never fires
            // here — Esc / Enter / click-outside remain the only ways out.
            if self.armed && !self.current_foreground_matches() {
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
