//! The card at its layout rect and the sprite painted over it (spec §1,
//! §10), driven headless through the real `NudgeApp` by `egui_kittest` as
//! `tests/kittest_card.rs` does. The harness has no OS window, so the
//! screen geometry is injected; what the real window does with it is
//! `tests/macos_native.rs`'s business.
//!
//! Node rects come back in physical pixels; every assertion here is in
//! points, so they are divided by the harness's scale first.

use eframe::egui::accesskit::Role;
use eframe::egui::{self, Rect, TextureOptions, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use nudge_web::app::NudgeApp;
use nudge_web::config::SpriteConfig;
use nudge_web::sprite::ScreenGeometry;
use nudge_web::sprite_png::{DecodedSprite, decode_rgba};

const PPP: f32 = 2.0;

const GREEN_PATH: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/electron/tests/e2e/fixtures/sprite-green.png"
);
const GREEN: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/electron/tests/e2e/fixtures/sprite-green.png"
));

/// This host: 1470×956 with a 33 pt menu bar.
fn g1() -> ScreenGeometry {
    ScreenGeometry {
        monitor: Rect::from_min_size(pos2(0.0, 0.0), vec2(1470.0, 956.0)),
        work: Rect::from_min_size(pos2(0.0, 33.0), vec2(1470.0, 833.0)),
    }
}

fn green_sprite() -> DecodedSprite {
    let (rgba, px) = decode_rgba(GREEN).expect("the green fixture decodes");
    DecodedSprite { rgba, px }
}

/// Our sprite's config: hanging above and left of a 679 pt card.
fn green_config() -> SpriteConfig {
    SpriteConfig {
        enabled: true,
        path: String::new(),
        x: -41,
        y: -277,
        card_width: 679,
    }
}

fn harness<'a>(
    size: egui::Vec2,
    build: impl FnOnce(&eframe::CreationContext<'_>) -> NudgeApp + 'a,
) -> Harness<'a, NudgeApp> {
    let mut harness = Harness::builder()
        .with_size(size)
        .with_pixels_per_point(PPP)
        .build_eframe(|cc| build(cc));
    harness.run();
    harness
}

/// The first field's rect in points. The card frame has no inner margin,
/// so the field's top-left corner is the card's.
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
fn plain_card_sits_where_the_shield_layout_says() {
    // 495 = floor((1470 − 480) / 2); 286 = floor(956 / 4) + 80 − 33.
    let harness = harness(vec2(1470.0, 833.0), |cc| {
        NudgeApp::new(cc, 10.0).with_screen_geometry(g1())
    });
    let field = first_field(&harness);
    assert_eq!(field.min, pos2(495.0, 286.0));
    assert_eq!(field.width(), 480.0);
}

#[test]
fn card_moves_by_the_sprite_layout_offset() {
    // The card widens to 679 and re-centres: 395 = floor((1470 − 679) / 2).
    // The sprite's top lands at 286 − 277 = 9, inside the window, so the
    // card keeps its anchor.
    let harness = harness(vec2(1470.0, 833.0), |cc| {
        NudgeApp::new(cc, 10.0)
            .with_screen_geometry(g1())
            .with_sprite(&cc.egui_ctx, green_sprite(), green_config())
    });
    let field = first_field(&harness);
    assert_eq!(field.min, pos2(395.0, 286.0));
    assert_eq!(field.width(), 480.0 + 199.0);
}

#[test]
fn sprite_uploads_one_nearest_texture() {
    // Native pixel density, never resampled (§10): NEAREST, at the PNG's size.
    let harness = harness(vec2(1470.0, 833.0), |cc| {
        NudgeApp::new(cc, 10.0)
            .with_screen_geometry(g1())
            .with_sprite(&cc.egui_ctx, green_sprite(), green_config())
    });
    assert_eq!(
        sprite_textures(&harness),
        vec![([720, 430], TextureOptions::NEAREST)]
    );
}

#[test]
fn no_sprite_uploads_nothing() {
    let harness = harness(vec2(1470.0, 833.0), |cc| {
        NudgeApp::new(cc, 10.0).with_screen_geometry(g1())
    });
    assert_eq!(sprite_textures(&harness), vec![]);
}

#[test]
fn without_injected_geometry_the_card_is_in_the_box() {
    // No screen to lay out on: the card sits in the window as it always
    // has — centred, top at a quarter of the height, 48 pt kept free.
    let harness = harness(vec2(520.0, 320.0), |cc| NudgeApp::new(cc, 10.0));
    let field = first_field(&harness);
    assert_eq!(field.min, pos2(24.0, 80.0));
    assert_eq!(field.width(), 472.0);
}

#[test]
fn a_sprite_change_that_fails_the_check_turns_the_sprite_off() {
    // A PNG that fails the check counts as off (§10): the texture goes and
    // the card is laid out plain again. The change is applied on close.
    let mut harness = harness(vec2(1470.0, 833.0), |cc| {
        NudgeApp::new(cc, 10.0)
            .with_screen_geometry(g1())
            .with_sprite(&cc.egui_ctx, green_sprite(), green_config())
    });
    assert_eq!(first_field(&harness).min.x, 395.0);

    harness.state_mut().set_pending_sprite(SpriteConfig {
        path: GREEN_PATH.replace("sprite-green", "no-such-file"),
        ..green_config()
    });
    harness.key_press(egui::Key::Escape);
    harness.run();

    assert_eq!(sprite_textures(&harness), vec![], "the sprite is off");
}

/// A primary click at `pos` (points), as the OS would deliver it.
fn click_at(harness: &Harness<'_, NudgeApp>, pos: egui::Pos2) {
    harness.event(egui::Event::PointerMoved(pos));
    for pressed in [true, false] {
        harness.event(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
}

#[test]
fn sprite_takes_no_clicks() {
    // The sprite is a painting, not a widget (§10): over the card a click
    // goes to the field beneath, off the card it is the click outside that
    // closes the popup (§4). The green sprite at (0, 0) is 360×215 pt, so it
    // covers rows 1–3 of the 130 pt card and 85 pt of air below it.
    let mut harness = harness(vec2(1470.0, 833.0), |cc| {
        NudgeApp::new(cc, 10.0)
            .with_screen_geometry(g1())
            .with_sprite(
                &cc.egui_ctx,
                green_sprite(),
                SpriteConfig {
                    x: 0,
                    y: 0,
                    ..green_config()
                },
            )
    });
    let card_top = first_field(&harness).top();
    assert_eq!(card_top, 286.0);

    // Row 2, under the sprite: the field takes the click.
    click_at(&harness, pos2(450.0, card_top + 60.0));
    harness.run();
    let inputs: Vec<_> = harness.get_all_by_role(Role::TextInput).collect();
    assert_eq!(inputs.len(), 3, "the popup is still up");
    assert!(inputs[1].is_focused(), "the click reached the second field");

    // The sprite's lower part, below the card: a click outside (§4).
    click_at(&harness, pos2(450.0, card_top + 130.0 + 40.0));
    harness.run();
    assert_eq!(harness.query_all_by_role(Role::TextInput).count(), 0);
}
