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

    // Native-only: tray communication
    #[cfg(not(target_arch = "wasm32"))]
    tray_show_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    #[cfg(not(target_arch = "wasm32"))]
    exit_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl NudgeApp {
    #[cfg(not(target_arch = "wasm32"))]
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        tray_show_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
        exit_flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        Self {
            doing: String::new(),
            bullshit: String::new(),
            next_minutes: "10".to_string(),
            focus_first: true,
            center_once: true,
            timer: Timer::new(std::time::Duration::from_secs(10 * 60)),
            trigger_source: TriggerSource::Timer,
            popup_visible: true, // show immediately on start
            tray_show_flag,
            exit_flag,
        }
    }

    #[cfg(target_arch = "wasm32")]
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
            popup_visible: true, // show immediately on start
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn now_timestamp() -> String {
        chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()
    }

    #[cfg(target_arch = "wasm32")]
    fn now_timestamp() -> String {
        // Use js_sys::Date for WASM timestamp
        let date = js_sys::Date::new_0();
        format!(
            "{}-{:02}-{:02}T{:02}:{:02}:{:02}",
            date.get_full_year(),
            date.get_month() + 1,
            date.get_date(),
            date.get_hours(),
            date.get_minutes(),
            date.get_seconds()
        )
    }

    fn show_popup(&mut self, _ctx: &egui::Context, source: TriggerSource) {
        self.trigger_source = source;
        self.popup_visible = true;
        self.focus_first = true;

        #[cfg(not(target_arch = "wasm32"))]
        _ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    }

    fn hide_popup(&mut self, _ctx: &egui::Context, action: Action) {
        // Write journal on submit
        if action == Action::Submit {
            let minutes: u32 = self.next_minutes.trim().parse().unwrap_or(10);
            let entry = crate::journal::JournalEntry {
                timestamp: Self::now_timestamp(),
                doing: self.doing.clone(),
                bullshit: self.bullshit.clone(),
                next_minutes: minutes,
            };

            #[cfg(not(target_arch = "wasm32"))]
            {
                // TODO: configurable path
                let path = std::path::PathBuf::from("journal.csv");
                crate::journal::write_entry(&path, &entry);
            }

            #[cfg(target_arch = "wasm32")]
            crate::journal::write_entry_to_localstorage(&entry);
        }

        // Shared: timer reset logic
        if nudge_state::should_reset_timer(self.trigger_source, action) {
            let interval = nudge_state::parse_interval(&self.next_minutes);
            self.timer.reset(interval);
        }

        // Shared: reset form
        self.doing.clear();
        self.bullshit.clear();
        self.focus_first = true;
        self.popup_visible = false;

        // Native-only: hide OS window
        #[cfg(not(target_arch = "wasm32"))]
        _ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
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

            ui.label(egui::RichText::new("Следующий nudge через (мин)").size(14.0).color(egui::Color32::from_gray(180)));
            ui.add_sized(
                [ui.available_width(), 28.0],
                egui::TextEdit::singleline(&mut self.next_minutes).font(egui::TextStyle::Body),
            );
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

            self.center_once = false;
        }

        // === Native-only: check tray/exit flags ===
        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.exit_flag.load(std::sync::atomic::Ordering::Relaxed) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                return;
            }

            if self.tray_show_flag.swap(false, std::sync::atomic::Ordering::Relaxed) {
                if !self.popup_visible {
                    self.show_popup(ctx, TriggerSource::Manual);
                }
            }
        }

        // === Shared: timer check ===
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
                self.draw_tray_screen(ctx, ui);
            }
        });
    }
}
