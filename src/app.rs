use eframe::egui;

pub struct NudgeApp;

impl NudgeApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        cc.egui_ctx.set_visuals(egui::Visuals::dark());
        Self
    }
}

impl eframe::App for NudgeApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Nudge works!");
            ui.label("Cross-compiled from WSL → Windows");
        });
    }
}
