//! A sprite change applied only while the popup is hidden (spec §10
//! "Applying a change"), headless through the real `NudgeApp`.
//!
//! Its own test binary for the same reason `macos_manual_open.rs` is: the
//! reopen goes through `platform::set_tray_clicked`, a process global that a
//! second harness in the same process could consume.

#![cfg(any(target_os = "macos", target_os = "windows"))]

use eframe::egui::accesskit::Role;
use eframe::egui::{self, Rect, TextureOptions, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use nudge_web::app::NudgeApp;
use nudge_web::config::SpriteConfig;
use nudge_web::sprite::ScreenGeometry;

const PPP: f32 = 2.0;

const GREEN_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/electron/tests/e2e/fixtures/sprite-green.png"
);

/// This host: 1470×956 with a 33 pt menu bar.
fn g1() -> ScreenGeometry {
    ScreenGeometry {
        monitor: Rect::from_min_size(pos2(0.0, 0.0), vec2(1470.0, 956.0)),
        work: Rect::from_min_size(pos2(0.0, 33.0), vec2(1470.0, 833.0)),
    }
}

fn first_field(harness: &Harness<'_, NudgeApp>) -> Rect {
    let px = harness
        .get_all_by_role(Role::TextInput)
        .next()
        .expect("the popup is up")
        .rect();
    Rect::from_min_max(px.min / PPP, px.max / PPP)
}

fn sprite_textures(harness: &Harness<'_, NudgeApp>) -> Vec<([usize; 2], TextureOptions)> {
    let manager = harness.ctx.tex_manager();
    let manager = manager.read();
    manager
        .allocated()
        .filter(|(_, meta)| meta.name == "nudge_sprite")
        .map(|(_, meta)| (meta.size, meta.options))
        .collect()
}

#[test]
fn a_sprite_change_while_visible_waits_for_hide() {
    let mut harness = Harness::builder()
        .with_size(vec2(1470.0, 833.0))
        .with_pixels_per_point(PPP)
        .build_eframe(|cc| NudgeApp::new(cc, 10.0).with_screen_geometry(g1()));
    harness.run();
    assert_eq!(first_field(&harness).min, pos2(495.0, 286.0));
    assert_eq!(sprite_textures(&harness), vec![]);

    // The change arrives while the card is up: nothing moves.
    harness.state_mut().set_pending_sprite(SpriteConfig {
        enabled: true,
        path: GREEN_PATH.to_string(),
        x: -41,
        y: -277,
        card_width: 679,
    });
    harness.run();
    assert_eq!(
        sprite_textures(&harness),
        vec![],
        "not under the user's hands"
    );
    assert_eq!(first_field(&harness).min, pos2(495.0, 286.0));

    // Esc closes it: the change is applied now, the texture replaced.
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert_eq!(
        sprite_textures(&harness),
        vec![([720, 430], TextureOptions::NEAREST)]
    );

    // The next open draws the re-laid-out card: 679 wide, re-centred.
    nudge_web::platform::set_tray_clicked();
    harness.run();
    let field = first_field(&harness);
    assert_eq!(field.min, pos2(395.0, 286.0));
    assert_eq!(field.width(), 679.0);
}
