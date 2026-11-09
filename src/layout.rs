//! Where the card and the sprite go (spec §1, §10): pure arithmetic in egui
//! points, top-left origin. No window and no I/O, so it builds on every target.

use eframe::egui::{Rect, Vec2, pos2, vec2};

use crate::config::SpriteConfig;

/// The card's height: the whole number of points that holds its three rows
/// and two separators (§2). A sprite changes only the width.
const CARD_HEIGHT: f32 = 130.0;
/// Where the card's top edge goes (§1): this fraction of the monitor's
/// height, plus the offset.
const CARD_TOP_FRACTION: f32 = 0.25;
/// The offset is where the card sat inside the legacy box: a quarter of the
/// box's height. So a card anchored on the monitor lands exactly where
/// `card_in_box` draws it in a box born at `window_position` — the two
/// paths describe one card.
const CARD_TOP_OFFSET: f32 = POPUP_SIZE.1 as f32 * CARD_TOP_FRACTION;
/// What a narrow window keeps free beside the card.
const CARD_SIDE_MARGIN: f32 = 48.0;

/// The spotlight window's size at creation, in points — the `(width, height)`
/// [`window_position`] centres. The visible size is the work area of the
/// monitor the popup is on ([`shield_layout`], spec §1); this box is only
/// what the window is born as, and on Windows, where the shield is still
/// pending, what it stays.
pub const POPUP_SIZE: (u32, u32) = (520, 320);

/// Screen position for the spotlight window: horizontally centered; the top
/// of the window sits at a quarter of the screen height (spec §1 — upper
/// third, at natural eye level). Window height is irrelevant to the y anchor.
// Only Windows still positions a box at creation; the macOS window is the
// work area (`shield_layout`), and the tests tie the two paths together.
pub fn window_position(screen: (u32, u32), window: (u32, u32)) -> (i32, i32) {
    let (sw, sh) = (screen.0 as i32, screen.1 as i32);
    let ww = window.0 as i32;
    let x = (sw - ww) / 2;
    let y = (sh as f32 * CARD_TOP_FRACTION).floor() as i32;
    (x, y)
}

/// The monitor the popup is on, in screen space: its full frame, and the
/// part free of the menu bar, Dock and taskbar. The work area is the window.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ScreenGeometry {
    pub monitor: Rect,
    pub work: Rect,
}

/// A sprite ready to place: its `x`/`y` from config, relative to the card's
/// top-left corner, its size in points (`sprite_points`), and the width the
/// card takes while it is on (§10).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpritePlacement {
    pub xy: (i32, i32),
    pub size: Vec2,
    pub card_width: f32,
}

/// Where to draw, in window coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PopupLayout {
    pub card: Rect,
    pub sprite: Option<Rect>,
}

/// A sprite is drawn at its native pixel density, 1 image pixel = 1 device
/// pixel: its size in points is its pixel size over the display's scale.
pub fn sprite_points(px: [u32; 2], pixels_per_point: f32) -> Vec2 {
    vec2(px[0] as f32, px[1] as f32) / pixels_per_point
}

/// A decoded sprite as the config places it, at this display's scale.
pub fn placement_of(cfg: &SpriteConfig, px: [u32; 2], pixels_per_point: f32) -> SpritePlacement {
    SpritePlacement {
        xy: (cfg.x, cfg.y),
        size: sprite_points(px, pixels_per_point),
        card_width: cfg.card_width as f32,
    }
}

/// The card's top edge on a monitor of this height (§1). Floored so it
/// agrees with [`window_position`]'s integer arithmetic.
fn anchor_top(monitor_height: f32) -> f32 {
    (monitor_height * CARD_TOP_FRACTION).floor() + CARD_TOP_OFFSET
}

/// The card inside a window that is *not* the work area — the kittest
/// harness and the browser canvas, which have no OS window: centred, top at
/// a quarter of the content height. The legacy 520×320 box puts it at
/// (24, 80), which is where `shield_layout` puts it on a real screen.
pub fn card_in_box(content: Rect) -> Rect {
    let width = (crate::config::DEFAULT_CARD_WIDTH as f32).min(content.width() - CARD_SIDE_MARGIN);
    let min = pos2(
        content.center().x - width / 2.0,
        content.min.y + content.height() * CARD_TOP_FRACTION,
    );
    Rect::from_min_size(min, vec2(width, CARD_HEIGHT))
}

/// The card at its §1 anchor on the monitor — centred, top at a quarter of
/// the height plus 80 pt — and the sprite relative to it, both in the
/// coordinates of a window that covers the work area. The card is its
/// default width, or the width the sprite's config asks for while the sprite
/// is on. A sprite that would rise above the work area moves the card down
/// by the overhang (§10); nothing is pinned at the bottom.
pub fn shield_layout(geo: &ScreenGeometry, sprite: Option<SpritePlacement>) -> PopupLayout {
    let card_width = sprite.map_or(crate::config::DEFAULT_CARD_WIDTH as f32, |s| s.card_width);
    let width = card_width.min(geo.work.width() - CARD_SIDE_MARGIN);
    let mut card_min = pos2(
        geo.monitor.min.x + ((geo.monitor.width() - width) / 2.0).floor(),
        geo.monitor.min.y + anchor_top(geo.monitor.height()),
    );
    if let Some(sprite) = sprite {
        let sprite_top = card_min.y + sprite.xy.1 as f32;
        card_min.y += (geo.work.min.y - sprite_top).max(0.0);
    }
    let card = Rect::from_min_size(card_min - geo.work.min.to_vec2(), vec2(width, CARD_HEIGHT));
    let sprite =
        sprite.map(|s| Rect::from_min_size(card.min + vec2(s.xy.0 as f32, s.xy.1 as f32), s.size));
    PopupLayout { card, sprite }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::SpriteConfig;
    use eframe::egui::{Rect, pos2, vec2};

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), vec2(w, h))
    }

    /// This host: 1470×956 with a 33 pt menu bar.
    fn g1() -> ScreenGeometry {
        ScreenGeometry {
            monitor: rect(0.0, 0.0, 1470.0, 956.0),
            work: rect(0.0, 33.0, 1470.0, 833.0),
        }
    }

    /// A short display: 1280×720 with a 25 pt menu bar.
    fn g2() -> ScreenGeometry {
        ScreenGeometry {
            monitor: rect(0.0, 0.0, 1280.0, 720.0),
            work: rect(0.0, 25.0, 1280.0, 695.0),
        }
    }

    /// Our sprite: 720×430 px on a 2x display, hanging above and left of a
    /// 679 pt card.
    fn our_sprite() -> SpritePlacement {
        SpritePlacement {
            xy: (-41, -277),
            size: vec2(360.0, 215.0),
            card_width: 679.0,
        }
    }

    /// A sprite beside a default-width card.
    fn placed(x: i32, y: i32, w: f32, h: f32) -> SpritePlacement {
        SpritePlacement {
            xy: (x, y),
            size: vec2(w, h),
            card_width: 480.0,
        }
    }

    #[test]
    fn sprite_points() {
        // 1 image pixel = 1 device pixel: an odd pixel count on 2x ends on a
        // half point rather than being resampled.
        assert_eq!(super::sprite_points([720, 430], 2.0), vec2(360.0, 215.0));
        assert_eq!(super::sprite_points([720, 430], 1.0), vec2(720.0, 430.0));
        assert_eq!(super::sprite_points([721, 431], 2.0), vec2(360.5, 215.5));
    }

    #[test]
    fn placement_of_the_green_fixture() {
        let cfg = SpriteConfig {
            enabled: true,
            path: String::new(),
            x: -41,
            y: -277,
            card_width: 679,
        };
        assert_eq!(placement_of(&cfg, [720, 430], 2.0), our_sprite());
        assert_eq!(
            placement_of(&cfg, [720, 430], 1.0),
            SpritePlacement {
                size: vec2(720.0, 430.0),
                ..our_sprite()
            }
        );
    }

    #[test]
    fn anchor_top() {
        // floor(h / 4) + 80 — integer parity with `window_position`.
        assert_eq!(super::anchor_top(956.0), 319.0);
        assert_eq!(super::anchor_top(957.0), 319.0);
        assert_eq!(super::anchor_top(1080.0), 350.0);
        assert_eq!(super::anchor_top(320.0), 160.0);
    }

    #[test]
    fn plain_card_is_centred_on_the_monitor_at_the_anchor() {
        // 495 = floor((1470 − 480) / 2); 286 = 319 − 33 (the window starts
        // where the work area does).
        let layout = shield_layout(&g1(), None);
        assert_eq!(layout.card, rect(495.0, 286.0, 480.0, 130.0));
        assert_eq!(layout.sprite, None);
    }

    #[test]
    fn window_local_coordinates_do_not_depend_on_where_the_monitor_is() {
        // A second display to the right of the first.
        let shifted = ScreenGeometry {
            monitor: g1().monitor.translate(vec2(1470.0, 0.0)),
            work: g1().work.translate(vec2(1470.0, 0.0)),
        };
        assert_eq!(
            shield_layout(&shifted, None).card,
            rect(495.0, 286.0, 480.0, 130.0)
        );
    }

    #[test]
    fn card_width_is_capped_by_the_work_area() {
        let narrow = ScreenGeometry {
            monitor: rect(0.0, 0.0, 500.0, 956.0),
            work: rect(0.0, 33.0, 500.0, 833.0),
        };
        assert_eq!(shield_layout(&narrow, None).card.width(), 452.0);
    }

    #[test]
    fn sprite_above_the_card_within_the_work_area_is_not_pinned() {
        // The sprite's top lands at 319 − 277 = 42 on the monitor, below the
        // 33 pt menu bar: the card stays at its anchor.
        let layout = shield_layout(&g1(), Some(our_sprite()));
        assert_eq!(layout.card, rect(395.0, 286.0, 679.0, 130.0));
        assert_eq!(layout.sprite, Some(rect(354.0, 9.0, 360.0, 215.0)));
    }

    #[test]
    fn sprite_overhanging_the_work_area_top_moves_the_card_down() {
        // Anchor 260; sprite top 260 − 277 = −17, above the 25 pt work-area
        // top by 42: the card moves down by that overhang and the sprite
        // starts at the window's top edge.
        let layout = shield_layout(&g2(), Some(our_sprite()));
        assert_eq!(layout.card, rect(300.0, 277.0, 679.0, 130.0));
        assert_eq!(layout.sprite, Some(rect(259.0, 0.0, 360.0, 215.0)));
    }

    #[test]
    fn sprite_below_or_beside_the_card_never_moves_it() {
        let below = shield_layout(&g1(), Some(placed(100, 200, 50.0, 40.0)));
        assert_eq!(below.card.min, pos2(495.0, 286.0));
        assert_eq!(below.sprite, Some(rect(595.0, 486.0, 50.0, 40.0)));

        let beside = shield_layout(&g1(), Some(placed(500, -10, 60.0, 50.0)));
        assert_eq!(beside.card.min, pos2(495.0, 286.0));
        assert_eq!(beside.sprite, Some(rect(995.0, 276.0, 60.0, 50.0)));
    }

    #[test]
    fn nothing_is_pinned_at_the_bottom() {
        // A sprite hanging far below the work area leaves the card alone and
        // simply runs off the window.
        let layout = shield_layout(&g2(), Some(placed(0, 400, 100.0, 900.0)));
        assert_eq!(layout.card, rect(400.0, 235.0, 480.0, 130.0));
        assert_eq!(layout.sprite, Some(rect(400.0, 635.0, 100.0, 900.0)));
    }

    #[test]
    fn sprite_size_stays_fractional() {
        let layout = shield_layout(&g1(), Some(placed(0, 130, 360.5, 215.5)));
        assert_eq!(layout.sprite.unwrap().size(), vec2(360.5, 215.5));
    }

    #[test]
    fn layout_is_never_negative_zero() {
        // The overhang case lands the sprite on y = 0; a monitor whose frame
        // was flipped from AppKit may carry a −0.0 origin. Neither may leak
        // a negative zero into the layout.
        let layout = shield_layout(&g2(), Some(our_sprite()));
        assert!(layout.sprite.unwrap().min.y.is_sign_positive());

        let flipped = ScreenGeometry {
            monitor: rect(-0.0, -0.0, 1280.0, 720.0),
            work: rect(-0.0, -0.0, 1280.0, 720.0),
        };
        let layout = shield_layout(
            &flipped,
            Some(SpritePlacement {
                card_width: 679.0,
                ..placed(-300, -260, 10.0, 10.0)
            }),
        );
        assert_eq!(layout.sprite.unwrap().min, pos2(0.0, 0.0));
        assert!(layout.sprite.unwrap().min.x.is_sign_positive());
        assert!(layout.sprite.unwrap().min.y.is_sign_positive());
    }

    #[test]
    fn shield_layout_on_the_legacy_box_is_todays_card() {
        // With the work area standing in for the old 520×320 box at
        // `window_position`, the shield arithmetic must reproduce the card
        // `card_in_box` draws in that box: the two paths agree.
        for w in [1470u32, 1471, 1920] {
            for h in [956u32, 957, 1080] {
                let (x, y) = window_position((w, h), POPUP_SIZE);
                let geo = ScreenGeometry {
                    monitor: rect(0.0, 0.0, w as f32, h as f32),
                    work: rect(x as f32, y as f32, POPUP_SIZE.0 as f32, POPUP_SIZE.1 as f32),
                };
                let want = rect(24.0, 80.0, 472.0, 130.0);
                assert_eq!(shield_layout(&geo, None).card, want, "{w}x{h}");
                assert_eq!(card_in_box(rect(0.0, 0.0, 520.0, 320.0)), want);
            }
        }
    }

    #[test]
    fn card_in_box_matches_the_wasm_canvas() {
        // e2e/tests/design-static.test.ts pins the card top at 150 on the
        // 600 pt canvas.
        for w in [528.0, 600.0, 1200.0] {
            assert_eq!(card_in_box(rect(0.0, 0.0, w, 600.0)).top(), 150.0);
        }
        assert_eq!(card_in_box(rect(0.0, 0.0, 600.0, 600.0)).width(), 480.0);
    }

    #[test]
    fn window_position_top_at_25pct_full_hd() {
        // 1920×1080, window 520×320 → x centered = (1920-520)/2 = 700;
        // top of window at 25% of 1080 = 270.
        assert_eq!(window_position((1920, 1080), (520, 320)), (700, 270));
    }

    #[test]
    fn window_position_top_at_25pct_small_screen() {
        // 1366×768, window 480×280 → x centered = (1366-480)/2 = 443;
        // top of window at 25% of 768 = 192.
        assert_eq!(window_position((1366, 768), (480, 280)), (443, 192));
    }

    #[test]
    fn position_is_computed_in_points_not_pixels() {
        // The Retina trap: a screen size that is already logical, so a
        // 1470x956 built-in display must produce the 1470x956 answer — not
        // the 2940x1912 one a `GetSystemMetrics`-shaped port would compute.
        let (x, y) = window_position((1470, 956), POPUP_SIZE);
        assert_eq!((x, y), (475, 239));
        let (x2, y2) = window_position((2940, 1912), POPUP_SIZE);
        assert_ne!(
            (x, y),
            (x2, y2),
            "if these agreed the test could not see the doubling it exists to catch"
        );
    }
}
