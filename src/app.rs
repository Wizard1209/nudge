use eframe::egui;

use crate::nudge_state::{self, Action, TriggerSource};
use crate::timer::Timer;

#[cfg(target_arch = "wasm32")]
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg(target_arch = "wasm32")]
static WEB_BLUR_FIRED: AtomicBool = AtomicBool::new(false);

/// Attach a window `blur` listener that sets a flag when the window loses focus.
/// Called once from `NudgeApp::new`; subsequent frames poll the flag.
#[cfg(target_arch = "wasm32")]
fn install_blur_listener() {
    use wasm_bindgen::prelude::Closure;
    use wasm_bindgen::JsCast;
    let Some(window) = web_sys::window() else { return };
    let closure = Closure::wrap(Box::new(move || {
        WEB_BLUR_FIRED.store(true, Ordering::Relaxed);
    }) as Box<dyn FnMut()>);
    let _ = window
        .add_event_listener_with_callback("blur", closure.as_ref().unchecked_ref());
    closure.forget();
}

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
    card_rect: Option<egui::Rect>,
    pill_rect: Option<egui::Rect>,
    was_focused: bool,

    // Native window handle for Win32 API calls
    #[cfg(target_os = "windows")]
    hwnd: Option<isize>,
}

impl NudgeApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut visuals = egui::Visuals::dark();
        // Transparent panel background so the card sits on top of wallpaper/desktop
        visuals.panel_fill = egui::Color32::TRANSPARENT;
        visuals.window_fill = egui::Color32::TRANSPARENT;
        cc.egui_ctx.set_visuals(visuals);

        #[cfg(target_arch = "wasm32")]
        install_blur_listener();

        Self {
            doing: String::new(),
            bullshit: String::new(),
            next_minutes: "10".to_string(),
            focus_first: true,
            center_once: true,
            timer: Timer::new(std::time::Duration::from_secs(10 * 60)),
            trigger_source: TriggerSource::Timer,
            popup_visible: true,
            error_message: None,
            card_rect: None,
            pill_rect: None,
            was_focused: false,
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

    fn show_popup(&mut self, _ctx: &egui::Context, source: TriggerSource) {
        self.trigger_source = source;
        self.popup_visible = true;
        self.focus_first = true;

        #[cfg(target_os = "windows")]
        if let Some(hwnd_val) = self.hwnd {
            unsafe {
                use windows::Win32::Foundation::HWND;
                use windows::Win32::UI::WindowsAndMessaging::*;
                let h = HWND(hwnd_val as *mut _);
                let _ = ShowWindow(h, SW_RESTORE);
                let _ = SetForegroundWindow(h);
            }
        }
    }

    fn hide_popup(&mut self, _ctx: &egui::Context, action: Action) {
        // Parse once so journal's minutes match the timer's actual interval
        // (parse_interval clamps ≤0 to 1 second, which must also be reflected in the log)
        let interval = nudge_state::parse_interval(&self.next_minutes);
        let minutes: f64 = interval.as_secs_f64() / 60.0;

        // Write journal on submit
        if action == Action::Submit {
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
                // TODO: resolve %USERPROFILE%\Documents\Nudge\journal-rust.ndjson
                let path = std::path::PathBuf::from("journal.ndjson");
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

        // Shared: reset form
        self.doing.clear();
        self.bullshit.clear();
        self.focus_first = true;
        self.popup_visible = false;

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

            // SW_HIDE stops update() loop, so schedule a thread to wake us up
            if should_reset {
                crate::tray_bridge::schedule_timer_wakeup(interval);
            }
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
        let screen = ctx.screen_rect();
        let card_width = 480.0_f32.min(screen.width() - 48.0);
        let top_offset = screen.height() * 0.30;

        let inner = egui::Area::new(egui::Id::new("nudge_card"))
            .anchor(egui::Align2::CENTER_TOP, egui::vec2(0.0, top_offset))
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
            ui.painter().rect_filled(
                row_rect,
                0.0,
                egui::Color32::from_white_alpha(10),
            );
        }
        ui.put(
            row_rect,
            egui::TextEdit::singleline(value)
                .id(field_id)
                .hint_text(hint)
                .frame(false)
                .margin(egui::Margin::symmetric(20, 12))
                .font(egui::FontId::proportional(16.0)),
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
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
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

            self.center_once = false;
        }

        // === Native-only: check tray event flags ===
        // Event handlers run on Windows message thread (set up in main.rs),
        // they restore the window + set flags. We check flags here.
        #[cfg(target_os = "windows")]
        {
            if crate::tray_bridge::is_exit_requested() {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }

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

        // === Shared: periodic repaint for timer ===
        ctx.request_repaint_after(std::time::Duration::from_secs(1));

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

            // Click outside card → dismiss (same as Esc)
            if let Some(rect) = self.card_rect {
                let clicked_outside = ctx.input(|i| {
                    i.pointer.any_click()
                        && i.pointer
                            .interact_pos()
                            .map_or(false, |p| !rect.contains(p))
                });
                if clicked_outside {
                    self.hide_popup(ctx, Action::Dismiss);
                    return;
                }
            }

            // Window focus-loss → dismiss (same as Esc).
            // Native: egui fills viewport().focused from winit events.
            // WASM: eframe doesn't populate it; we poll a DOM-blur flag instead.
            #[cfg(not(target_arch = "wasm32"))]
            {
                let focused_now = ctx.input(|i| i.viewport().focused);
                if focused_now == Some(true) {
                    self.was_focused = true;
                }
                if self.was_focused && focused_now == Some(false) {
                    self.hide_popup(ctx, Action::Dismiss);
                    return;
                }
            }
            #[cfg(target_arch = "wasm32")]
            if WEB_BLUR_FIRED.swap(false, Ordering::Relaxed) {
                self.hide_popup(ctx, Action::Dismiss);
                return;
            }
        }

        // === Shared: render UI ===
        // Transparent central panel so wallpaper / desktop shows through around card.
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE)
            .show(ctx, |_ui| {});

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
