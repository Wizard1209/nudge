#[cfg(target_arch = "wasm32")]
mod wasm_entry {
    use crate::app::NudgeApp;
    use crate::autostart::FakeProvider;
    use crate::config;
    use crate::settings_app::{SettingsApp, SettingsHost};
    use wasm_bindgen::prelude::*;

    fn page_wants_settings() -> bool {
        let Some(window) = web_sys::window() else {
            return false;
        };
        let location = window.location();
        let search = location.search().unwrap_or_default();
        let pathname = location.pathname().unwrap_or_default();
        search.contains("settings") || pathname.contains("settings")
    }

    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        let web_options = eframe::WebOptions::default();

        let document = web_sys::window().unwrap().document().unwrap();
        let canvas = document
            .get_element_by_id("nudge_canvas")
            .unwrap()
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .unwrap();

        if page_wants_settings() {
            wasm_bindgen_futures::spawn_local(async move {
                let (cfg, err) = config::load_from_localstorage();
                if let Some(e) = err {
                    web_sys::console::warn_1(&format!("[nudge-settings] {e}").into());
                }
                let host = SettingsHost {
                    persist: Box::new(|c: &config::Config| config::save_to_localstorage(c)),
                    ..SettingsHost::headless(Box::new(FakeProvider::new(cfg.autostart)))
                };

                eframe::WebRunner::new()
                    .start(
                        canvas,
                        web_options,
                        Box::new(move |cc| Ok(Box::new(SettingsApp::new(cc, cfg, host)))),
                    )
                    .await
                    .expect("failed to start settings eframe");
            });
            return Ok(());
        }

        wasm_bindgen_futures::spawn_local(async move {
            eframe::WebRunner::new()
                .start(
                    canvas,
                    web_options,
                    Box::new(|cc| {
                        Ok(Box::new(NudgeApp::new(
                            cc,
                            config::DEFAULT_INTERVAL_MINUTES,
                        )))
                    }),
                )
                .await
                .expect("failed to start eframe");
        });

        Ok(())
    }
}

pub mod app;
pub mod autostart;
pub mod config;
pub mod config_watcher;
#[cfg(not(target_arch = "wasm32"))]
pub mod daisy;
pub mod hotkey;
pub mod journal;
pub mod layout;
pub mod nudge_state;
#[cfg(not(target_arch = "wasm32"))]
pub mod platform;
pub mod recorder;
pub mod screen_pick;
pub mod settings_app;
#[cfg(not(target_arch = "wasm32"))]
pub mod sprite_png;
pub mod timer;
#[cfg(target_os = "windows")]
pub mod tray_bridge;
#[cfg(not(target_arch = "wasm32"))]
pub mod update;
pub mod word_jump;
