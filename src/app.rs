use eframe::egui;

use crate::nudge_state::{self, Action, TriggerSource};
use crate::timer::Timer;

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

    // Native window handle for Win32 API calls
    #[cfg(target_os = "windows")]
    hwnd: Option<isize>,
}

impl NudgeApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
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

    fn draw_form(&mut self, ui: &mut egui::Ui) {
        let margin = egui::Margin::symmetric(24, 20);
        egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
            ui.style_mut().spacing.item_spacing.y = 6.0;

            ui.label(egui::RichText::new("Что я делаю?").size(14.0).color(egui::Color32::from_gray(180)));
            let doing_response = ui.add_sized(
                [ui.available_width(), 28.0],
                egui::TextEdit::singleline(&mut self.doing).font(egui::TextStyle::Body),
            );
            if self.focus_first {
                doing_response.request_focus();
                self.focus_first = false;
            }

            ui.add_space(6.0);

            ui.label(egui::RichText::new("Не хуйню ли я делаю?").size(14.0).color(egui::Color32::from_gray(180)));
            ui.add_sized(
                [ui.available_width(), 28.0],
                egui::TextEdit::singleline(&mut self.bullshit).font(egui::TextStyle::Body),
            );

            ui.add_space(6.0);

            ui.label(egui::RichText::new("Следующий nudge через (мін)").size(14.0).color(egui::Color32::from_gray(180)));
            ui.add_sized(
                [ui.available_width(), 28.0],
                egui::TextEdit::singleline(&mut self.next_minutes).font(egui::TextStyle::Body),
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
        });
    }

    fn draw_tray_screen(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let margin = egui::Margin::symmetric(24, 20);
        egui::Frame::NONE.inner_margin(margin).show(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(40.0);

                let remaining = self.timer.remaining();
                let mins = remaining.as_secs() / 60;
                let secs = remaining.as_secs() % 60;
                ui.label(
                    egui::RichText::new(format!("Next nudge in {}:{:02}", mins, secs))
                        .size(22.0)
                        .color(egui::Color32::from_gray(200)),
                );

                ui.add_space(16.0);

                if ui.button(egui::RichText::new("Nudge now").size(14.0)).clicked() {
                    self.show_popup(ctx, TriggerSource::Manual);
                }
            });
        });
    }
}

impl eframe::App for NudgeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // === First frame setup ===
        if self.center_once {
            ctx.set_visuals(egui::Visuals::dark());

            #[cfg(not(target_arch = "wasm32"))]
            if let Some(cmd) = egui::ViewportCommand::center_on_screen(ctx) {
                ctx.send_viewport_cmd(cmd);
            }

            // Extract native window handle and share with tray event handlers
            #[cfg(target_os = "windows")]
            {
                use raw_window_handle::HasWindowHandle;
                if let Ok(wh) = _frame.window_handle() {
                    if let raw_window_handle::RawWindowHandle::Win32(h) = wh.as_raw() {
                        let hwnd_val = h.hwnd.get();
                        self.hwnd = Some(hwnd_val);
                        crate::tray_bridge::store_hwnd(hwnd_val);
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
        }

        // === Shared: render UI ===
        egui::CentralPanel::default().show(ctx, |ui| {
            if self.popup_visible {
                self.draw_form(ui);
            } else {
                // WASM: show countdown + "Nudge now" button on canvas
                // Native: window is SW_HIDE'd, nothing to render
                #[cfg(target_arch = "wasm32")]
                self.draw_tray_screen(ctx, ui);
            }
        });
    }
}
