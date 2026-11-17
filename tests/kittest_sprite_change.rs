#![cfg(any(target_os = "macos", target_os = "windows"))]

use std::sync::{Mutex, MutexGuard};

use eframe::egui::accesskit::Role;
use eframe::egui::{self, Rect, TextureId, TextureOptions, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use nudge_web::app::NudgeApp;
use nudge_web::config::SpriteConfig;
use nudge_web::layout::ScreenGeometry;

const PPP: f32 = 2.0;

static SEAM: Mutex<()> = Mutex::new(());

fn seam() -> MutexGuard<'static, ()> {
    SEAM.lock().unwrap_or_else(|e| e.into_inner())
}

const GREEN_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/electron/tests/e2e/fixtures/sprite-green.png"
);

fn manual_open() {
    nudge_web::platform::request_manual_open();
}

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

fn sprite_ids(harness: &Harness<'_, NudgeApp>) -> Vec<TextureId> {
    let manager = harness.ctx.tex_manager();
    let manager = manager.read();
    manager
        .allocated()
        .filter(|(_, meta)| meta.name == "nudge_sprite")
        .map(|(id, _)| *id)
        .collect()
}

fn green(x: i32) -> SpriteConfig {
    SpriteConfig {
        enabled: true,
        path: GREEN_PATH.to_string(),
        x,
        y: -277,
        card_width: 679,
    }
}

fn freed_by_one_frame(harness: &mut Harness<'_, NudgeApp>) -> Vec<TextureId> {
    harness.step();
    harness.output().textures_delta.free.clone()
}

#[test]
fn a_sprite_change_while_visible_waits_for_hide() {
    let _seam = seam();
    let mut harness = Harness::builder()
        .with_size(vec2(1470.0, 833.0))
        .with_pixels_per_point(PPP)
        .build_eframe(|cc| NudgeApp::new(cc, 10.0).with_screen_geometry(g1()));
    harness.run();
    manual_open();
    harness.run();
    assert_eq!(first_field(&harness).min, pos2(495.0, 286.0));
    assert_eq!(sprite_textures(&harness), vec![]);

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

    harness.key_press(egui::Key::Escape);
    harness.run();
    assert_eq!(sprite_textures(&harness), vec![]);

    manual_open();
    harness.run();
    let field = first_field(&harness);
    assert_eq!(field.min, pos2(395.0, 286.0));
    assert_eq!(field.width(), 679.0);
    assert_eq!(
        sprite_textures(&harness),
        vec![([720, 430], TextureOptions::NEAREST)]
    );
}

/// eframe applies a frame's texture changes only on a frame it paints, and it
/// paints none while the popup is hidden. A sprite replaced then must not free
/// its old texture on that frame — the free would be dropped with it and the
/// texture would stay on the GPU for the life of the process. The harness
/// paints every frame, so what it can check is the frame the free is on: not
/// a hidden one, and not the reveal frame either — eframe runs only `logic`
/// there, the window being hidden when the frame began.
#[test]
fn a_sprite_replaced_while_hidden_is_freed_on_a_painted_frame() {
    let _seam = seam();
    let mut harness = Harness::builder()
        .with_size(vec2(1470.0, 833.0))
        .with_pixels_per_point(PPP)
        .with_step_dt(1.0 / 60.0)
        .build_eframe(|cc| NudgeApp::new(cc, 10.0).with_screen_geometry(g1()));
    harness.run();
    manual_open();
    harness.run();

    harness.state_mut().set_pending_sprite(green(-41));
    harness.key_press(egui::Key::Escape);
    harness.run();
    manual_open();
    harness.run();
    let old = sprite_ids(&harness);
    assert_eq!(old.len(), 1, "the first sprite is on the GPU");

    harness.key_press(egui::Key::Escape);
    harness.run();
    harness.state_mut().set_pending_sprite(green(-40));
    for frame in 0..3 {
        assert_eq!(
            freed_by_one_frame(&mut harness),
            vec![],
            "hidden frame {frame} is not painted: a free on it is lost"
        );
    }

    manual_open();
    assert_eq!(freed_by_one_frame(&mut harness), vec![], "the reveal frame");
    assert_eq!(
        freed_by_one_frame(&mut harness),
        old,
        "the first frame that starts with the popup up"
    );
    harness.run();
    let new = sprite_ids(&harness);
    assert_eq!(new.len(), 1, "one sprite texture, the new one");
    assert_ne!(new, old);
}

#[test]
fn a_sprite_change_that_fails_the_check_turns_the_sprite_off() {
    let _seam = seam();
    let mut harness = Harness::builder()
        .with_size(vec2(1470.0, 833.0))
        .with_pixels_per_point(PPP)
        .build_eframe(|cc| NudgeApp::new(cc, 10.0).with_screen_geometry(g1()));
    harness.run();
    manual_open();
    harness.run();
    harness.state_mut().set_pending_sprite(green(-41));
    harness.key_press(egui::Key::Escape);
    harness.run();
    manual_open();
    harness.run();
    assert_eq!(first_field(&harness).min.x, 395.0);
    assert_eq!(sprite_ids(&harness).len(), 1);

    harness.state_mut().set_pending_sprite(SpriteConfig {
        path: GREEN_PATH.replace("sprite-green", "no-such-file"),
        ..green(-41)
    });
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert_eq!(sprite_ids(&harness).len(), 1, "not freed on a hidden frame");
    manual_open();
    harness.run();

    assert_eq!(sprite_textures(&harness), vec![], "the sprite is off");
    assert_eq!(first_field(&harness).min.x, 495.0, "the card is plain");
}
