//! 64×64 RGBA marguerite icon renderer for the system tray.
//!
//! A Bellis perennis ("marguerite"): 12 plump elliptical petals around a
//! bright yellow center, white at the base fading to a soft pink at the
//! outermost tip — the signature look that distinguishes a marguerite from
//! a plain field daisy. Petals fall one by one as the nudge timer counts down
//! (petal 0 at 12 o'clock falls first, then petal 1, …, clockwise).
//! `DriftState` animates the just-fallen petal radially outward + slightly
//! downward (gravity) while fading.
//!
//! At 64×64 the icon is provided at the largest size the Windows shell
//! normally requests for a notification-area icon. Windows will downscale
//! for lower-DPI trays.

use std::f32::consts::PI;
use std::time::Duration;

pub const ICON_SIZE: u32 = 64;
pub const PETAL_COUNT: u8 = 12;

const CENTER: f32 = (ICON_SIZE as f32 - 1.0) * 0.5;

/// Distance from flower center to the center of each petal ellipse.
const PETAL_ORBIT: f32 = 20.0;
/// Half-length of a petal along its long (radial) axis.
const PETAL_LEN: f32 = 11.0;
/// Half-width of a petal along its short (tangential) axis. Wider than a
/// classic field daisy — gives the plump rounded look that reads as a
/// marguerite / tamagotchi-style flower.
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

/// The daisy state for one tick of the countdown, derived purely from the
/// configured interval (`duration`) and how much of it has `elapsed`.
///
/// Spec §5 timing lives here: how many petals are still attached, the petal
/// currently drifting away (if any), and whether enough time has passed —
/// including the final drift — to fire the popup. The tray thread reads
/// `elapsed` from the wall clock, calls [`frame_at`], then turns this into a
/// `set_icon` + the `TIMER_FIRED` atomic. Keeping it pure lets the §5
/// invariants run under `cargo test` without the Win32 message pump.
#[derive(Clone, Copy, Debug)]
pub struct DaisyFrame {
    /// Petals still attached to the flower (0..=PETAL_COUNT). Feed straight
    /// into [`render`]; excludes any petal animating in `drift`.
    pub petals_remaining: u8,
    /// The just-fallen petal mid-animation, or `None` outside a drift window.
    pub drift: Option<DriftState>,
    /// `true` once `elapsed` has reached `duration` plus the final drift —
    /// the moment the popup should fire (spec §5: pop only after the last
    /// petal has finished falling).
    pub should_fire: bool,
    /// Length of one petal's lifetime (`duration / PETAL_COUNT`). Exposed so
    /// the tray loop can schedule its next wake until the next petal drop.
    pub petal_duration: Duration,
    /// Time since the most recent petal dropped. Pairs with `petal_duration`
    /// for the tray loop's pre-drop wake scheduling.
    pub time_since_drop: Duration,
}

/// Compute the daisy state for `elapsed` into an interval of length
/// `duration` (spec §5 timing). Pure: no clock, no I/O.
///
/// Petals fall one per `duration / PETAL_COUNT`, in index order (0 at 12
/// o'clock, clockwise). The freshly-dropped petal drifts for `drift_dur`
/// — at most 250 ms, and never more than half a petal's lifetime so a
/// drifting petal can't overrun the next drop on very short intervals.
/// `elapsed` is allowed to grow past `duration` so the final drift can play
/// out before `should_fire` flips.
pub fn frame_at(duration: Duration, elapsed: Duration) -> DaisyFrame {
    let petal_count_u64 = PETAL_COUNT as u64;
    let petal_dur_nanos = (duration.as_nanos() as u64).max(1) / petal_count_u64;
    let petal_dur = Duration::from_nanos(petal_dur_nanos);
    let elapsed_nanos = elapsed.as_nanos() as u64;
    let raw_drops = elapsed_nanos / petal_dur_nanos;
    let petals_dropped = raw_drops.min(petal_count_u64) as u8;
    let petals_remaining = PETAL_COUNT - petals_dropped;
    let time_since_drop =
        Duration::from_nanos(elapsed_nanos - (petals_dropped as u64) * petal_dur_nanos);
    // Drift gets at most half a petal lifetime, so it can't run into
    // the next drop even at silly-short intervals.
    let drift_dur = Duration::from_millis(250).min(petal_dur / 2);

    let drift = if petals_dropped > 0 && time_since_drop < drift_dur {
        let progress =
            (time_since_drop.as_secs_f32() / drift_dur.as_secs_f32()).clamp(0.0, 1.0);
        Some(DriftState {
            petal_index: petals_dropped - 1,
            progress,
        })
    } else {
        None
    };

    // Fire only after the very last drift has finished, so the user sees the
    // final petal fall before the popup takes focus.
    let should_fire = elapsed >= duration + drift_dur;

    DaisyFrame {
        petals_remaining,
        drift,
        should_fire,
        petal_duration: petal_dur,
        time_since_drop,
    }
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

    // ---- frame_at: spec §5 timeline invariants ----------------------------
    //
    // Expected values are derived from the same integer/float arithmetic
    // frame_at runs, to pin CURRENT behavior. With PETAL_COUNT = 12 and a
    // 120 s interval, one petal falls every 10 s and the drift window is
    // min(250 ms, 10 s / 2) = 250 ms.

    const DUR_120S: Duration = Duration::from_secs(120);

    #[test]
    fn mid_interval_steady_state_has_no_drift() {
        // 35 s in: 35 / 10 = 3 petals dropped, 9 remaining; 5 s since the
        // last drop, well past the 250 ms drift window → no drift.
        let f = frame_at(DUR_120S, Duration::from_secs(35));
        assert_eq!(f.petals_remaining, 9);
        assert!(f.drift.is_none(), "5 s after a drop is past the drift window");
        assert!(!f.should_fire);
        assert_eq!(f.petal_duration, Duration::from_secs(10));
        assert_eq!(f.time_since_drop, Duration::from_secs(5));
    }

    #[test]
    fn just_after_petal_drops_has_drift_near_zero() {
        // Exactly 30 s in: the 3rd petal (index 2) has just detached. Drift
        // present with progress 0.0 and petals_remaining already decremented.
        let f = frame_at(DUR_120S, Duration::from_secs(30));
        assert_eq!(f.petals_remaining, 9);
        let d = f.drift.expect("a petal just dropped → drift present");
        assert_eq!(d.petal_index, 2);
        assert!(d.progress.abs() < 1e-6, "progress ≈ 0 at the instant of drop");
        assert_eq!(f.time_since_drop, Duration::ZERO);
        assert!(!f.should_fire);
    }

    #[test]
    fn fires_only_after_final_drift_completes() {
        // drift_dur = 250 ms, so should_fire flips at duration + 250 ms.
        let just_before = frame_at(DUR_120S, DUR_120S + Duration::from_millis(200));
        assert!(!just_before.should_fire, "200 ms past expiry < 250 ms drift");

        let just_after = frame_at(DUR_120S, DUR_120S + Duration::from_millis(300));
        assert!(just_after.should_fire, "300 ms past expiry ≥ 250 ms drift");
        // All petals gone by expiry; the count caps at PETAL_COUNT.
        assert_eq!(just_after.petals_remaining, 0);

        // Boundary is inclusive (elapsed >= duration + drift_dur).
        let exact = frame_at(DUR_120S, DUR_120S + Duration::from_millis(250));
        assert!(exact.should_fire);
    }

    #[test]
    fn short_interval_clamps_drift_before_next_petal() {
        // 1.2 s interval → petal_dur = 100 ms, drift_dur = min(250, 50) = 50 ms.
        // The drift window must close before the next petal drops at 100 ms.
        let dur = Duration::from_millis(1200);

        // 20 ms after the 1st drop: inside the 50 ms drift window.
        let inside = frame_at(dur, Duration::from_millis(120));
        assert_eq!(inside.petals_remaining, 11);
        let d = inside.drift.expect("20 ms after a drop is inside drift");
        assert_eq!(d.petal_index, 0);

        // 60 ms after the 1st drop: past the 50 ms clamp but the next petal
        // (at 100 ms) hasn't dropped yet → no drift, still 11 remaining.
        let after = frame_at(dur, Duration::from_millis(160));
        assert_eq!(after.petals_remaining, 11);
        assert!(
            after.drift.is_none(),
            "drift must clamp shut before the next petal drops"
        );
        assert_eq!(after.petal_duration, Duration::from_millis(100));
    }

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
