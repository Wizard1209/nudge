#[cfg(target_arch = "wasm32")]
mod wasm_entry {
    use crate::app::NudgeApp;
    use crate::autostart::FakeProvider;
    use crate::config;
    use crate::settings_app::{PersistFn, SettingsApp};
    use wasm_bindgen::prelude::*;

    /// True when the page wants the settings UI instead of the main popup.
    /// We accept the marker in *either* the query string (`?settings`) or
    /// the path (`/settings.html`) so a static-server setup that doesn't
    /// rewrite query strings still routes correctly. The Puppeteer e2e
    /// test for settings hits `settings.html` which matches the path
    /// branch; native testing via `?settings` matches the query branch.
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
            // Settings UI on the same canvas. Provider is a fake (no
            // registry in the browser); persistence writes the same
            // localStorage key the native code reads when running under
            // wasm-bindgen (`nudge-config`). This is what the e2e
            // settings.test.ts asserts against.
            wasm_bindgen_futures::spawn_local(async move {
                let (cfg, err) = config::load_from_localstorage();
                if let Some(e) = err {
                    web_sys::console::warn_1(&format!("[nudge-settings] {e}").into());
                }
                let provider: Box<dyn crate::autostart::AutostartProvider> =
                    Box::new(FakeProvider::new(cfg.autostart));
                let persist: PersistFn = Box::new(|c: &config::Config| {
                    config::save_to_localstorage(c)
                });

                eframe::WebRunner::new()
                    .start(
                        canvas,
                        web_options,
                        Box::new(move |cc| {
                            Ok(Box::new(SettingsApp::new(cc, cfg, provider, persist)))
                        }),
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
                    // WASM has no on-disk config; the popup's minutes field
                    // is initialised to the same 10-minute default the
                    // native build's config defaults to.
                    Box::new(|cc| Ok(Box::new(NudgeApp::new(cc, 10.0)))),
                )
                .await
                .expect("failed to start eframe");
        });

        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
mod app;
pub mod autostart;
pub mod config;
pub mod config_watcher;
#[cfg(target_os = "windows")]
pub mod daisy;
pub mod hotkey;
pub mod journal;
pub mod nudge_state;
pub mod settings_app;
pub mod timer;
pub mod word_jump;
#[cfg(target_os = "windows")]
pub mod tray_bridge;
