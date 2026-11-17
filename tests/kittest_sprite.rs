use eframe::egui::accesskit::Role;
use eframe::egui::{self, Rect, TextureOptions, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;

use nudge_web::app::NudgeApp;
use nudge_web::config::SpriteConfig;
use nudge_web::layout::ScreenGeometry;
use nudge_web::sprite_png::{DecodedSprite, decode_rgba};

const PPP: f32 = 2.0;

const GREEN: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/electron/tests/e2e/fixtures/sprite-green.png"
));

fn g1() -> ScreenGeometry {
    ScreenGeometry {
        monitor: Rect::from_min_size(pos2(0.0, 0.0), vec2(1470.0, 956.0)),
        work: Rect::from_min_size(pos2(0.0, 33.0), vec2(1470.0, 833.0)),
    }
}

fn green_sprite() -> DecodedSprite {
    decode_rgba(GREEN).expect("the green fixture decodes")
}

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
    harness.state_mut().deliver_deadline();
    harness.run();
    harness
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
fn plain_card_sits_where_the_shield_layout_says() {
    let harness = harness(vec2(1470.0, 833.0), |cc| {
        NudgeApp::new(cc, 10.0).with_screen_geometry(g1())
    });
    let field = first_field(&harness);
    assert_eq!(field.min, pos2(495.0, 286.0));
    assert_eq!(field.width(), 480.0);
}

#[test]
fn card_moves_by_the_sprite_layout_offset() {
    let harness = harness(vec2(1470.0, 833.0), |cc| {
        NudgeApp::new(cc, 10.0)
            .with_screen_geometry(g1())
            .with_sprite(green_sprite(), green_config())
    });
    let field = first_field(&harness);
    assert_eq!(field.min, pos2(395.0, 286.0));
    assert_eq!(field.width(), 480.0 + 199.0);
}

#[test]
fn sprite_uploads_one_nearest_texture() {
    let harness = harness(vec2(1470.0, 833.0), |cc| {
        NudgeApp::new(cc, 10.0)
            .with_screen_geometry(g1())
            .with_sprite(green_sprite(), green_config())
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
    let harness = harness(vec2(520.0, 320.0), |cc| NudgeApp::new(cc, 10.0));
    let field = first_field(&harness);
    assert_eq!(field.min, pos2(24.0, 80.0));
    assert_eq!(field.width(), 472.0);
}

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
    let mut harness = harness(vec2(1470.0, 833.0), |cc| {
        NudgeApp::new(cc, 10.0)
            .with_screen_geometry(g1())
            .with_sprite(
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

    click_at(&harness, pos2(450.0, card_top + 60.0));
    harness.run();
    let inputs: Vec<_> = harness.get_all_by_role(Role::TextInput).collect();
    assert_eq!(inputs.len(), 3, "the popup is still up");
    assert!(inputs[1].is_focused(), "the click reached the second field");

    click_at(&harness, pos2(450.0, card_top + 130.0 + 40.0));
    harness.run();
    assert_eq!(harness.query_all_by_role(Role::TextInput).count(), 0);
}
