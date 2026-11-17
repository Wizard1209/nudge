use eframe::egui::{Pos2, Rect};

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

fn contains(r: &Rect, p: Pos2) -> bool {
    p.x >= r.min.x && p.x < r.max.x && p.y >= r.min.y && p.y < r.max.y
}

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

    fn two() -> [Rect; 2] {
        [
            rect(0.0, 0.0, 1470.0, 956.0),
            rect(1470.0, 0.0, 1920.0, 1080.0),
        ]
    }

    #[test]
    fn the_front_window_s_screen_is_chosen() {
        let front = rect(1600.0, 100.0, 800.0, 600.0);
        assert_eq!(pick_screen(Some(front), pos2(10.0, 10.0), &two()), 1);
    }

    #[test]
    fn a_straddling_front_window_counts_where_its_centre_is() {
        let right_leaning = rect(1200.0, 100.0, 800.0, 600.0);
        assert_eq!(
            pick_screen(Some(right_leaning), pos2(10.0, 10.0), &two()),
            1
        );
        let left_leaning = rect(900.0, 100.0, 800.0, 600.0);
        assert_eq!(
            pick_screen(Some(left_leaning), pos2(1800.0, 10.0), &two()),
            0
        );
    }

    #[test]
    fn the_pointer_is_ignored_while_there_is_a_front_window() {
        let front = rect(1600.0, 100.0, 800.0, 600.0);
        let pointer_on_first = pos2(700.0, 400.0);
        assert_eq!(pick_screen(Some(front), pointer_on_first, &two()), 1);
        let front = rect(100.0, 100.0, 800.0, 600.0);
        let pointer_on_second = pos2(2000.0, 400.0);
        assert_eq!(pick_screen(Some(front), pointer_on_second, &two()), 0);
    }

    #[test]
    fn a_front_window_centred_on_no_screen_goes_where_it_overlaps_most() {
        let front = rect(1000.0, 900.0, 1200.0, 400.0);
        assert_eq!(pick_screen(Some(front), pos2(10.0, 10.0), &two()), 1);
    }

    #[test]
    fn without_a_front_window_the_pointer_s_screen_is_chosen() {
        assert_eq!(pick_screen(None, pos2(2000.0, 500.0), &two()), 1);
        assert_eq!(pick_screen(None, pos2(700.0, 500.0), &two()), 0);
    }

    #[test]
    fn with_nothing_to_go_on_the_primary_is_chosen() {
        assert_eq!(pick_screen(None, pos2(-500.0, -500.0), &two()), 0);
        assert_eq!(pick_screen(None, pos2(5000.0, 5000.0), &two()), 0);
    }
}
