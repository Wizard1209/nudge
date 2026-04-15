use eframe::egui;

pub struct NudgeApp {
    doing: String,
    bullshit: String,
    next_minutes: String,
    focus_first: bool,
}

impl NudgeApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        Self {
            doing: String::new(),
            bullshit: String::new(),
            next_minutes: "10".to_string(),
            focus_first: true,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn save_and_close(&self, ctx: &egui::Context) {
        let minutes: u32 = self.next_minutes.trim().parse().unwrap_or(10);
        let entry = crate::journal::JournalEntry {
            timestamp: chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
            doing: self.doing.clone(),
            bullshit: self.bullshit.clone(),
            next_minutes: minutes,
        };

        // TODO: configurable path (next to exe or %APPDATA%)
        let path = std::path::PathBuf::from("journal.csv");
        crate::journal::write_entry(&path, &entry);

        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    #[cfg(target_arch = "wasm32")]
    fn save_and_close(&self, _ctx: &egui::Context) {
        // WASM: no file I/O, no-op for now (used in e2e visual tests only)
    }
}

impl eframe::App for NudgeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Handle global keys before UI
        let enter_pressed = ctx.input(|i| i.key_pressed(egui::Key::Enter));
        let esc_pressed = ctx.input(|i| i.key_pressed(egui::Key::Escape));

        if enter_pressed {
            self.save_and_close(ctx);
            return;
        }
        if esc_pressed {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.vertical(|ui| {
                ui.label("Что я делаю?");
                let doing_response = ui.text_edit_singleline(&mut self.doing);
                if self.focus_first {
                    doing_response.request_focus();
                    self.focus_first = false;
                }

                ui.add_space(8.0);

                ui.label("Не хуйню ли я делаю?");
                ui.text_edit_singleline(&mut self.bullshit);

                ui.add_space(8.0);

                ui.label("Следующий nudge через (мин)");
                ui.text_edit_singleline(&mut self.next_minutes);
            });
        });
    }
}
