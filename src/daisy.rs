//! 64×64 RGBA marguerite icon renderer for the system tray.
//!
//! A Bellis perennis ("marguerite"): 12 plump elliptical petals around a
//! bright yellow center, white at the base fading to a soft pink at the
//! outermost tip — the signature look that distinguishes маргаритка from a
//! plain field daisy. Petals fall one by one as the nudge timer counts down
//! (petal 0 at 12 o'clock falls first, then petal 1, …, clockwise).
//! `DriftState` animates the just-fallen petal radially outward + slightly
//! downward (gravity) while fading.
//!
//! At 64×64 the icon is provided at the largest size the Windows shell
//! normally requests for a notification-area icon. Windows will downscale
//! for lower-DPI trays.

use std::f32::consts::PI;

pub const ICON_SIZE: u32 = 64;
pub const PETAL_COUNT: u8 = 12;

const CENTER: f32 = (ICON_SIZE as f32 - 1.0) * 0.5;

/// Distance from flower center to the center of each petal ellipse.
const PETAL_ORBIT: f32 = 20.0;
/// Half-length of a petal along its long (radial) axis.
const PETAL_LEN: f32 = 11.0;
/// Half-width of a petal along its short (tangential) axis. Wider than a
/// classic field daisy — gives the plump rounded look that reads as a
/// маргаритка / tamagotchi-style flower.
const PETAL_WIDTH: f32 = 4.6;
/// Radius of the central yellow disk.
const CENTER_RADIUS: f32 = 6.5;

const COLOR_PETAL_BASE: [u8; 3] = [248, 248, 252];
/// Soft warm pink at the petal tip — the marguerite signature.
const COLOR_PETAL_TIP: [u8; 3] = [240, 130, 165];
const COLOR_CENTER: [u8; 3] = [255, 200, 55];

/// A petal in the middle of falling away from the flower.
#[derive(Clone, Copy, Debug)]
pub struct DriftState {
    /// Index of the drifting petal. 0 = 12 o'clock, increasing clockwise.
    pub petal_index: u8,
    /// 0.0 = just detached (still in place), 1.0 = fully gone.
    pub progress: f32,
}

/// Render one frame of the daisy as a 64×64 RGBA buffer.
///
/// `petals_remaining` counts petals still attached (0..=12); does NOT
/// include any petal currently animating in `drift`.
pub fn render(petals_remaining: u8, drift: Option<DriftState>) -> Vec<u8> {
    let petals_remaining = petals_remaining.min(PETAL_COUNT);
    let mut buf = vec![0u8; (ICON_SIZE * ICON_SIZE * 4) as usize];

    // Petals fall in order starting from index 0 → with N remaining,
    // indices [PETAL_COUNT - N .. PETAL_COUNT) are still on the flower.
    let first_static = PETAL_COUNT - petals_remaining;

    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            let px = x as f32;
            let py = y as f32;

            // Pick the petal covering this pixel most strongly so the tip
            // colour comes from the *correct* petal (their bounding boxes
            // overlap slightly near the inner ring).
            let mut best_alpha: f32 = 0.0;
            let mut best_color: [u8; 3] = COLOR_PETAL_BASE;
            for i in first_static..PETAL_COUNT {
                let (a, c) = petal_sample(i, 0.0, 0.0, 1.0, px, py);
                if a > best_alpha {
                    best_alpha = a;
                    best_color = c;
                }
            }
            if let Some(d) = drift {
                if d.petal_index < PETAL_COUNT {
                    let radial_off = d.progress * 8.0;
                    // Slight gravity arc — petals drift outward + downward.
                    let gravity_y = d.progress * d.progress * 6.0;
                    let fade = (1.0 - d.progress).clamp(0.0, 1.0);
                    let (a, c) =
                        petal_sample(d.petal_index, radial_off, gravity_y, fade, px, py);
                    if a > best_alpha {
                        best_alpha = a;
                        best_color = c;
                    }
                }
            }

            // Yellow center disk sits on top of the petals.
            let dx = px - CENTER;
            let dy = py - CENTER;
            let r = (dx * dx + dy * dy).sqrt();
            let center_a = soft_disk(r, CENTER_RADIUS);

            let (rr, gg, bb, aa) =
                composite(best_color, best_alpha, COLOR_CENTER, center_a);
            let idx = ((y * ICON_SIZE + x) * 4) as usize;
            buf[idx] = rr;
            buf[idx + 1] = gg;
            buf[idx + 2] = bb;
            buf[idx + 3] = aa;
        }
    }

    buf
}

fn petal_angle(i: u8) -> f32 {
    (i as f32) * (2.0 * PI / PETAL_COUNT as f32)
}

/// Returns (alpha, color) at pixel (px, py) for petal `i`. Tip colour
/// blends white→pink along the outward half of the petal.
fn petal_sample(
    i: u8,
    radial_off: f32,
    gravity_off: f32,
    alpha_scale: f32,
    px: f32,
    py: f32,
) -> (f32, [u8; 3]) {
    let theta = petal_angle(i);
    // Radial outward direction; y+ is down on screen.
    let dir_x = theta.sin();
    let dir_y = -theta.cos();
    let perp_x = -dir_y;
    let perp_y = dir_x;

    let cx = CENTER + dir_x * (PETAL_ORBIT + radial_off);
    let cy = CENTER + dir_y * (PETAL_ORBIT + radial_off) + gravity_off;

    let vx = px - cx;
    let vy = py - cy;
    let r_par = vx * dir_x + vy * dir_y;
    let r_per = vx * perp_x + vy * perp_y;

    // Ellipse SDF with a soft 1px falloff at the edge for anti-aliasing.
    let n = (r_par / PETAL_LEN).powi(2) + (r_per / PETAL_WIDTH).powi(2);
    let inside = 1.0 - n;
    let a = (inside * 4.0 + 0.4).clamp(0.0, 1.0) * alpha_scale;

    // Tip colouring: blend toward pink along the outward half. r_par > 0
    // means "this pixel is on the side of the petal pointing away from the
    // flower center". Squaring concentrates the pink near the very tip.
    let tip_factor = (r_par / PETAL_LEN).max(0.0).powi(2);
    let color = lerp_rgb(COLOR_PETAL_BASE, COLOR_PETAL_TIP, tip_factor);

    (a, color)
}

fn soft_disk(r: f32, radius: f32) -> f32 {
    (radius - r + 0.5).clamp(0.0, 1.0)
}

fn lerp_rgb(a: [u8; 3], b: [u8; 3], t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        (a[0] as f32 * (1.0 - t) + b[0] as f32 * t) as u8,
        (a[1] as f32 * (1.0 - t) + b[1] as f32 * t) as u8,
        (a[2] as f32 * (1.0 - t) + b[2] as f32 * t) as u8,
    ]
}

/// "Above over below" alpha compositing (yellow center over white petals).
fn composite(
    below: [u8; 3],
    a_below: f32,
    above: [u8; 3],
    a_above: f32,
) -> (u8, u8, u8, u8) {
    let a_out = a_above + a_below * (1.0 - a_above);
    if a_out <= 0.001 {
        return (0, 0, 0, 0);
    }
    let mix = |c_b: u8, c_a: u8| -> u8 {
        let v = (c_a as f32 * a_above + c_b as f32 * a_below * (1.0 - a_above)) / a_out;
        v.clamp(0.0, 255.0) as u8
    };
    (
        mix(below[0], above[0]),
        mix(below[1], above[1]),
        mix(below[2], above[2]),
        (a_out * 255.0).clamp(0.0, 255.0) as u8,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alpha_at(buf: &[u8], x: u32, y: u32) -> u8 {
        let i = ((y * ICON_SIZE + x) * 4 + 3) as usize;
        buf[i]
    }

    #[test]
    fn full_daisy_has_top_petal_opaque() {
        let buf = render(PETAL_COUNT, None);
        // Top petal centered around y ≈ 31.5 - 20 = 11.5.
        // Vertical centerline at x = 32.
        assert!(alpha_at(&buf, 32, 8) > 100, "top petal should be opaque");
    }

    #[test]
    fn top_petal_tip_is_pink() {
        let buf = render(PETAL_COUNT, None);
        // Near the outer tip of the top petal (y ≈ 11.5 - PETAL_LEN ≈ 0.5)
        let i = ((3 * ICON_SIZE + (CENTER as u32)) * 4) as usize;
        let r = buf[i];
        let g = buf[i + 1];
        let b = buf[i + 2];
        let a = buf[i + 3];
        assert!(a > 60, "tip should be at least partially opaque");
        // Pink ≈ (240, 130, 165): R high, G clearly less than R, B between.
        assert!(r as i32 - g as i32 > 30, "tip should be warmer than white");
    }

    #[test]
    fn empty_daisy_has_only_center() {
        let buf = render(0, None);
        // Outer region — transparent.
        assert_eq!(alpha_at(&buf, 32, 2), 0);
        // Dead center — opaque yellow.
        let i = ((CENTER as u32 * ICON_SIZE + CENTER as u32) * 4) as usize;
        assert!(buf[i + 3] > 200);
        assert!(buf[i] > 200);
        assert!(buf[i + 1] > 150);
        assert!(buf[i + 2] < 120);
    }

    #[test]
    fn drift_at_progress_zero_matches_attached_petal() {
        let a = render(PETAL_COUNT, None);
        let b = render(
            PETAL_COUNT - 1,
            Some(DriftState { petal_index: 0, progress: 0.0 }),
        );
        for y in 2..16 {
            for x in 26..38 {
                let i = ((y * ICON_SIZE + x) * 4 + 3) as usize;
                assert!(
                    (a[i] as i32 - b[i] as i32).abs() <= 4,
                    "alpha mismatch at ({x},{y}): {} vs {}",
                    a[i],
                    b[i]
                );
            }
        }
    }

    #[test]
    fn drift_fades_at_progress_one() {
        let buf = render(
            PETAL_COUNT - 1,
            Some(DriftState { petal_index: 0, progress: 1.0 }),
        );
        assert!(alpha_at(&buf, 32, 8) < 10);
    }

    /// Eyeball helper. Run with `--ignored --nocapture` to print all 13
    /// petal counts plus a mid-drift frame as ASCII.
    #[test]
    #[ignore]
    fn ascii_dump_all_states() {
        for petals in (0..=PETAL_COUNT).rev() {
            println!("\n=== {} petals remaining ===", petals);
            dump_ascii(&render(petals, None));
        }
        println!("\n=== drift (petal 0, progress=0.5) ===");
        dump_ascii(&render(
            PETAL_COUNT - 1,
            Some(DriftState { petal_index: 0, progress: 0.5 }),
        ));
    }

    fn dump_ascii(buf: &[u8]) {
        for y in 0..ICON_SIZE {
            let mut row = String::new();
            for x in 0..ICON_SIZE {
                let i = ((y * ICON_SIZE + x) * 4) as usize;
                let a = buf[i + 3];
                let r = buf[i];
                let g = buf[i + 1];
                let b = buf[i + 2];
                let ch = if a < 32 {
                    ' '
                } else if r > 200 && g > 150 && b < 120 {
                    if a > 200 { '@' } else { 'o' }
                } else if r as i32 - g as i32 > 30 {
                    // Pink-ish
                    if a > 200 { 'P' } else if a > 100 { 'p' } else { '.' }
                } else {
                    if a > 200 { '#' } else if a > 100 { '*' } else { '.' }
                };
                row.push(ch);
            }
            println!("{}", row);
        }
    }
}
