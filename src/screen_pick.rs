//! Which monitor the popup takes (spec §1): pure arithmetic over rects that
//! some platform read, in one coordinate system. No window and no I/O, so it
//! builds on every target, and a second display can be a test fixture on a
//! host that has one.
//!
//! The rule, from the spec: "Which monitor: the one with the keyboard focus
//! — the display holding the frontmost application's front window, as
//! Spotlight chooses — read when the popup is shown, on every trigger alike.
//! With no such window it is the display under the pointer; the pointer's
//! display on its own is the valid alternative, not the rule."

use eframe::egui::{Pos2, Rect};

/// The index into `screens` of the display the popup goes on.
///
/// * `front` — the frontmost application's front window, if there is one;
/// * `pointer` — where the cursor is;
/// * `screens` — every display's frame, the primary first (the one at the
///   origin, as `NSScreen.screens` lists them). Non-empty.
///
/// In order: the screen containing the front window's centre; else, for a
/// front window whose centre is on no screen, the one it overlaps most; else
/// the screen under the pointer; else the primary. Bounds are half-open, so
/// a point on the seam between two displays belongs to the one on the right
/// (or below), never to both.
pub fn pick_screen(front: Option<Rect>, pointer: Pos2, screens: &[Rect]) -> usize {
    if let Some(front) = front {
        if let Some(i) = screens.iter().position(|s| contains(s, front.center())) {
            return i;
        }
        let most_overlap = screens
            .iter()
            .enumerate()
            .map(|(i, s)| (i, overlap(s, &front)))
            .filter(|(_, area)| *area > 0.0)
            .max_by(|a, b| a.1.total_cmp(&b.1));
        if let Some((i, _)) = most_overlap {
            return i;
        }
    }
    screens
        .iter()
        .position(|s| contains(s, pointer))
        .unwrap_or(0)
}

/// Half-open on the far edges: a point on the seam belongs to the display
/// that starts there.
fn contains(r: &Rect, p: Pos2) -> bool {
    p.x >= r.min.x && p.x < r.max.x && p.y >= r.min.y && p.y < r.max.y
}

/// The area the two rects share, zero when they only touch.
fn overlap(a: &Rect, b: &Rect) -> f32 {
    let w = a.max.x.min(b.max.x) - a.min.x.max(b.min.x);
    let h = a.max.y.min(b.max.y) - a.min.y.max(b.min.y);
    if w > 0.0 && h > 0.0 { w * h } else { 0.0 }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{pos2, vec2};

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(pos2(x, y), vec2(w, h))
    }

    /// A laptop panel at the origin and a larger display to its right, the
    /// seam at x = 1470.
    fn two() -> [Rect; 2] {
        [
            rect(0.0, 0.0, 1470.0, 956.0),
            rect(1470.0, 0.0, 1920.0, 1080.0),
        ]
    }

    /// Spec §1: "the display holding the frontmost application's front
    /// window" — a window wholly on the second screen puts the popup there.
    #[test]
    fn the_front_window_s_screen_is_chosen() {
        let front = rect(1600.0, 100.0, 800.0, 600.0);
        assert_eq!(pick_screen(Some(front), pos2(10.0, 10.0), &two()), 1);
    }

    /// Spec §1: a front window straddling the seam is "on" the display that
    /// holds its centre, the way Spotlight reads it.
    #[test]
    fn a_straddling_front_window_counts_where_its_centre_is() {
        // 1200..2000 — centre at 1600, on the second screen.
        let right_leaning = rect(1200.0, 100.0, 800.0, 600.0);
        assert_eq!(
            pick_screen(Some(right_leaning), pos2(10.0, 10.0), &two()),
            1
        );
        // 900..1700 — centre at 1300, on the first.
        let left_leaning = rect(900.0, 100.0, 800.0, 600.0);
        assert_eq!(
            pick_screen(Some(left_leaning), pos2(1800.0, 10.0), &two()),
            0
        );
    }

    /// Spec §1: the front window decides; the pointer is only the fallback
    /// "with no such window", so where it sits changes nothing here.
    #[test]
    fn the_pointer_is_ignored_while_there_is_a_front_window() {
        let front = rect(1600.0, 100.0, 800.0, 600.0);
        let pointer_on_first = pos2(700.0, 400.0);
        assert_eq!(pick_screen(Some(front), pointer_on_first, &two()), 1);
        let front = rect(100.0, 100.0, 800.0, 600.0);
        let pointer_on_second = pos2(2000.0, 400.0);
        assert_eq!(pick_screen(Some(front), pointer_on_second, &two()), 0);
    }

    /// A front window whose centre is on no screen at all — hanging mostly
    /// below the shorter panel, say — goes with the display it overlaps most,
    /// still before the pointer gets a say.
    #[test]
    fn a_front_window_centred_on_no_screen_goes_where_it_overlaps_most() {
        // 1000..2200 wide, 900..1300 tall: centre (1600, 1100) is below the
        // second screen's 1080 and right of the first; the overlap with the
        // second (730×180) beats the first's (470×56).
        let front = rect(1000.0, 900.0, 1200.0, 400.0);
        assert_eq!(pick_screen(Some(front), pos2(10.0, 10.0), &two()), 1);
    }

    /// Spec §1: "With no such window it is the display under the pointer."
    #[test]
    fn without_a_front_window_the_pointer_s_screen_is_chosen() {
        assert_eq!(pick_screen(None, pos2(2000.0, 500.0), &two()), 1);
        assert_eq!(pick_screen(None, pos2(700.0, 500.0), &two()), 0);
    }

    /// No front window and the pointer on no display either (a stale
    /// position after a display was unplugged): the primary, so the popup
    /// is never placed off every screen.
    #[test]
    fn with_nothing_to_go_on_the_primary_is_chosen() {
        assert_eq!(pick_screen(None, pos2(-500.0, -500.0), &two()), 0);
        assert_eq!(pick_screen(None, pos2(5000.0, 5000.0), &two()), 0);
    }
}
