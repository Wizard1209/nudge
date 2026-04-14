#[cfg(target_arch = "wasm32")]
mod wasm_entry {
    use crate::app::NudgeApp;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen(start)]
    pub fn start() -> Result<(), JsValue> {
        let web_options = eframe::WebOptions::default();

        let document = web_sys::window().unwrap().document().unwrap();
        let canvas = document
            .get_element_by_id("nudge_canvas")
            .unwrap()
            .dyn_into::<web_sys::HtmlCanvasElement>()
            .unwrap();

        wasm_bindgen_futures::spawn_local(async move {
            eframe::WebRunner::new()
                .start(
                    canvas,
                    web_options,
                    Box::new(|cc| Ok(Box::new(NudgeApp::new(cc)))),
                )
                .await
                .expect("failed to start eframe");
        });

        Ok(())
    }
}

mod app;
