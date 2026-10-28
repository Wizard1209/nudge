//! A sprite change applied only while the popup is hidden (spec §10
//! "Applying a change"), headless through the real `NudgeApp`.
//!
//! Its own test binary for the same reason `macos_manual_open.rs` is: the
//! reopen goes through the platform seam's process-global flags, which a
//! second harness in the same process could consume. The tests here take
//! [`SEAM`] for the same reason.

#![cfg(any(target_os = "macos", target_os = "windows"))]

use std::sync::{Mutex, MutexGuard};

use eframe::egui::accesskit::Role;
use eframe::egui::{self, Rect, TextureId, TextureOptions, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use nudge_web::app::NudgeApp;
use nudge_web::config::SpriteConfig;
use nudge_web::sprite::ScreenGeometry;

const PPP: f32 = 2.0;

/// Held for a whole test: the platform seam's flags are per process.
static SEAM: Mutex<()> = Mutex::new(());

fn seam() -> MutexGuard<'static, ()> {
    SEAM.lock().unwrap_or_else(|e| e.into_inner())
}

const GREEN_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/electron/tests/e2e/fixtures/sprite-green.png"
);

/// A status-item click or the hotkey, raised where the OS raises it: on
/// macOS the flag `tick` turns into a reveal, on Windows the tray thread's
/// edge.
fn manual_open() {
    #[cfg(target_os = "macos")]
    nudge_web::platform::request_manual_open();
    #[cfg(target_os = "windows")]
    nudge_web::platform::set_tray_clicked();
}

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

/// Run exactly one frame and return the textures it freed.
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

    // Esc closes it: the change is applied now. The texture waits for a
    // frame that is painted — eframe drops uploads from hidden ones.
    harness.key_press(egui::Key::Escape);
    harness.run();
    assert_eq!(sprite_textures(&harness), vec![]);

    // The next open draws the re-laid-out card, 679 wide and re-centred,
    // and the sprite over it.
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

    // A sprite applied at a close, and uploaded by the next open.
    harness.state_mut().set_pending_sprite(green(-41));
    harness.key_press(egui::Key::Escape);
    harness.run();
    manual_open();
    harness.run();
    let old = sprite_ids(&harness);
    assert_eq!(old.len(), 1, "the first sprite is on the GPU");

    // Closed again, and the sprite moves while nobody is looking.
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

    // The next open paints, and that is where the old texture goes.
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

/// A PNG that fails the check counts as off (§10): the texture goes and the
/// card is laid out plain again. The change is applied on close; its texture
/// is still allocated after that hidden close and gone after the next open,
/// the first frame eframe paints.
#[test]
fn a_sprite_change_that_fails_the_check_turns_the_sprite_off() {
    let _seam = seam();
    let mut harness = Harness::builder()
        .with_size(vec2(1470.0, 833.0))
        .with_pixels_per_point(PPP)
        .build_eframe(|cc| NudgeApp::new(cc, 10.0).with_screen_geometry(g1()));
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
