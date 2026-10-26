//! RGBA marguerite icon renderer for the system tray / menu bar.
//!
//! A Bellis perennis ("marguerite"): 12 plump petals around a bright yellow
//! center, white at the base fading to a soft pink at the outermost tip —
//! the signature look that distinguishes a marguerite from a plain field
//! daisy. Each petal is a strip that runs out from under the centre (spec
//! §5): narrowest where it leaves the disc, widest about three fifths of
//! the way out, closed by a pointed-oval tip, so petal and centre meet with
//! no gap. Petals fall one by one as the nudge timer counts down
//! (petal 0 at 12 o'clock falls first, then petal 1, …, clockwise).
//! `DriftState` animates the just-fallen petal radially outward + slightly
//! downward (gravity) while fading.
//!
//! Two entry points draw the same flower:
//!
//! - [`render`] is the Windows path: a fixed 64×64 bitmap, the largest size
//!   the Windows shell normally requests for a notification-area icon.
//!   Windows downscales it itself for lower-DPI trays.
//! - [`render_at`] draws the flower at any pixel size, for hosts that take
//!   one bitmap per display density and show each as-is (the macOS status
//!   item: 18 px at 1x, 36 px at 2x). Spec §5, "Drawn at the size the host
//!   gives…": nothing between the renderer and the screen resamples the
//!   daisy. Every length is the 64 px design scaled by `size / 64`, and each
//!   pixel is supersampled so a petal a pixel and a half wide still has a
//!   graded edge rather than a staircase.

use std::f32::consts::PI;
use std::time::Duration;

pub const ICON_SIZE: u32 = 64;
pub const PETAL_COUNT: u8 = 12;

/// Sub-samples per axis per pixel in [`render_at`]. 4×4 is what the Electron
/// twin uses; 16 coverage levels are plenty for an 18 px edge.
const SUPERSAMPLE: u32 = 4;

// The flower is designed on a 64 px canvas; `Geometry::at` scales it.

/// Where a petal starts, from the flower centre: under the disc, so a
/// falling petal's base shows only once it has slid out.
const PETAL_BASE: f32 = 3.0;
/// Where a petal's tip ends, from the flower centre.
const PETAL_TIP: f32 = 31.0;
/// Full half-width of a petal across its body. Wider than a classic field
/// daisy — gives the plump rounded look that reads as a marguerite /
/// tamagotchi-style flower.
const PETAL_WIDTH: f32 = 4.6;
/// The strip's half-width along the petal as a share of [`PETAL_WIDTH`],
/// sampled every 5 % of the visible length from the disc edge to the tip:
/// the median profile of the ray florets traced on a Bellis perennis photo
/// (eight florets lying over both neighbours, so both edges were real). Half
/// as wide at the disc, full at 60 %, then the pointed-oval tip.
const PETAL_PROFILE: [f32; 21] = [
    0.479, 0.479, 0.479, 0.504, 0.555, 0.592, 0.623, 0.718, 0.822, 0.875, 0.919, 0.965, 1.000,
    0.972, 0.931, 0.908, 0.858, 0.818, 0.723, 0.504, 0.000,
];
/// Radius of the central yellow disk.
const CENTER_RADIUS: f32 = 6.5;
/// How far a falling petal travels outward over the whole drift.
const DRIFT_OUTWARD: f32 = 8.0;
/// How far a falling petal sags under "gravity" over the whole drift.
const DRIFT_GRAVITY: f32 = 6.0;
/// Width of the centre disk's edge ramp.
const DISK_RAMP: f32 = 0.5;

/// The 64 px design scaled to one output size. All lengths in output pixels.
#[derive(Clone, Copy, Debug)]
struct Geometry {
    /// Flower centre, in whichever pixel coordinate convention the caller
    /// samples with (see [`render`] vs [`render_at`]).
    center: f32,
    petal_base: f32,
    petal_tip: f32,
    petal_width: f32,
    center_radius: f32,
    drift_outward: f32,
    drift_gravity: f32,
    disk_ramp: f32,
}

impl Geometry {
    /// Geometry for a `size`-px bitmap sampled in continuous coordinates
    /// (pixel `x` spans `x..x+1`, so the flower centre sits at `size / 2`).
    fn at(size: u32) -> Self {
        let k = size as f32 / ICON_SIZE as f32;
        Self {
            center: size as f32 * 0.5,
            petal_base: PETAL_BASE * k,
            petal_tip: PETAL_TIP * k,
            petal_width: PETAL_WIDTH * k,
            center_radius: CENTER_RADIUS * k,
            drift_outward: DRIFT_OUTWARD * k,
            drift_gravity: DRIFT_GRAVITY * k,
            disk_ramp: DISK_RAMP * k,
        }
    }

    /// The Windows 64 px bitmap samples each pixel at its integer coordinate,
    /// so its centre is `(64 - 1) / 2`. Scale factor 1, hence every length
    /// is the design constant itself.
    #[cfg_attr(target_os = "macos", allow(dead_code))] // the Windows tray's bitmap; macOS draws per density
    fn windows() -> Self {
        Self {
            center: (ICON_SIZE as f32 - 1.0) * 0.5,
            ..Self::at(ICON_SIZE)
        }
    }

    /// Where petal `i` (offset by a drift) is rooted and how it is oriented:
    /// the frame's origin is the flower centre, moved by the drift, and the
    /// petal runs out from it along `dir`.
    fn petal_frame(&self, i: u8, radial_off: f32, gravity_off: f32) -> PetalFrame {
        let theta = petal_angle(i);
        // Radial outward direction; y+ is down on screen.
        let dir_x = theta.sin();
        let dir_y = -theta.cos();
        PetalFrame {
            cx: self.center + dir_x * radial_off,
            cy: self.center + dir_y * radial_off + gravity_off,
            dir_x,
            dir_y,
        }
    }
}

/// One petal's root and radial direction, resolved once per petal rather
/// than once per sample.
#[derive(Clone, Copy, Debug)]
struct PetalFrame {
    cx: f32,
    cy: f32,
    dir_x: f32,
    dir_y: f32,
}

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
        let progress = (time_since_drop.as_secs_f32() / drift_dur.as_secs_f32()).clamp(0.0, 1.0);
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

/// Render one frame of the daisy as a 64×64 RGBA buffer — the Windows path.
///
/// `petals_remaining` counts petals still attached (0..=12); does NOT
/// include any petal currently animating in `drift`.
///
/// One sample per pixel at the pixel's integer coordinate, with the edge
/// softness coming from the analytic falloff in [`petal_sample`] alone. Kept
/// as its own loop rather than `render_at(64, ..)`: supersampling changes
/// the edge bytes, and this bitmap is pinned byte-for-byte by its tests.
#[cfg_attr(target_os = "macos", allow(dead_code))] // the Windows tray's bitmap; macOS draws per density
pub fn render(petals_remaining: u8, drift: Option<DriftState>) -> Vec<u8> {
    let g = Geometry::windows();
    let scene = Scene::new(&g, petals_remaining, drift);
    let mut buf = vec![0u8; (ICON_SIZE * ICON_SIZE * 4) as usize];

    for y in 0..ICON_SIZE {
        for x in 0..ICON_SIZE {
            let (r, gg, b, a) = scene.sample(&g, x as f32, y as f32);
            let idx = ((y * ICON_SIZE + x) * 4) as usize;
            buf[idx] = r as u8;
            buf[idx + 1] = gg as u8;
            buf[idx + 2] = b as u8;
            buf[idx + 3] = a as u8;
        }
    }

    buf
}

/// Render one frame of the daisy as a `size`×`size` straight-alpha RGBA
/// buffer, for hosts that draw one bitmap per display density (spec §5).
///
/// Same flower as [`render`] with every length scaled by `size / 64`. Each
/// pixel is the premultiplied average of a [`SUPERSAMPLE`]² grid of samples
/// across it, so an edge crossing the pixel leaves partial alpha instead of
/// an in/out decision — at 18 px a petal is about a pixel and a half wide
/// and would otherwise be a staircase.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))] // only the macOS status item draws per density
pub fn render_at(size: u32, petals_remaining: u8, drift: Option<DriftState>) -> Vec<u8> {
    let g = Geometry::at(size);
    let scene = Scene::new(&g, petals_remaining, drift);
    let mut buf = vec![0u8; (size * size * 4) as usize];

    for y in 0..size {
        for x in 0..size {
            let (r, gg, b, a) = scene.pixel_supersampled(&g, x, y);
            let idx = ((y * size + x) * 4) as usize;
            buf[idx] = r as u8;
            buf[idx + 1] = gg as u8;
            buf[idx + 2] = b as u8;
            buf[idx + 3] = a as u8;
        }
    }

    buf
}

/// Everything about one frame that does not depend on the sample point:
/// which petals are attached, where the drifting one is, how faded it is.
struct Scene {
    /// Attached petals: indices `[first_static, PETAL_COUNT)`.
    first_static: u8,
    /// The drifting petal, resolved to its frame and alpha scale.
    drift: Option<(PetalFrame, f32)>,
}

impl Scene {
    fn new(g: &Geometry, petals_remaining: u8, drift: Option<DriftState>) -> Self {
        let petals_remaining = petals_remaining.min(PETAL_COUNT);
        // Petals fall in order starting from index 0 → with N remaining,
        // indices [PETAL_COUNT - N .. PETAL_COUNT) are still on the flower.
        let first_static = PETAL_COUNT - petals_remaining;
        let drift = drift.filter(|d| d.petal_index < PETAL_COUNT).map(|d| {
            let radial_off = d.progress * g.drift_outward;
            // Slight gravity arc — petals drift outward + downward.
            let gravity_y = d.progress * d.progress * g.drift_gravity;
            let fade = (1.0 - d.progress).clamp(0.0, 1.0);
            (g.petal_frame(d.petal_index, radial_off, gravity_y), fade)
        });
        Self {
            first_static,
            drift,
        }
    }

    /// Colour and alpha of the composited flower at one point:
    /// `(r, g, b)` in `0..=255`, `a` in `0..=255`, all already clamped so a
    /// bare `as u8` quantizes them.
    fn sample(&self, g: &Geometry, px: f32, py: f32) -> (f32, f32, f32, f32) {
        // Pick the petal covering this point most strongly so the tip
        // colour comes from the *correct* petal (their bounding boxes
        // overlap slightly near the inner ring).
        let mut best_alpha: f32 = 0.0;
        let mut best_color: [u8; 3] = COLOR_PETAL_BASE;
        for i in self.first_static..PETAL_COUNT {
            let frame = g.petal_frame(i, 0.0, 0.0);
            let (a, c) = petal_sample(g, &frame, 1.0, px, py);
            if a > best_alpha {
                best_alpha = a;
                best_color = c;
            }
        }
        if let Some((frame, fade)) = &self.drift {
            let (a, c) = petal_sample(g, frame, *fade, px, py);
            if a > best_alpha {
                best_alpha = a;
                best_color = c;
            }
        }

        // Yellow center disk sits on top of the petals.
        let dx = px - g.center;
        let dy = py - g.center;
        let r = (dx * dx + dy * dy).sqrt();
        let center_a = soft_disk(r, g.center_radius, g.disk_ramp);

        composite(best_color, best_alpha, COLOR_CENTER, center_a)
    }

    /// Pixel `(x, y)` as the premultiplied average of a `SUPERSAMPLE`²
    /// grid of samples across it, so an edge that crosses the pixel leaves
    /// partial alpha. Same `(r, g, b, a)` convention as [`Scene::sample`].
    fn pixel_supersampled(&self, g: &Geometry, x: u32, y: u32) -> (f32, f32, f32, f32) {
        let (mut r, mut gg, mut b, mut a) = (0.0f32, 0.0f32, 0.0f32, 0.0f32);
        let step = 1.0 / SUPERSAMPLE as f32;
        for j in 0..SUPERSAMPLE {
            for i in 0..SUPERSAMPLE {
                let sx = x as f32 + (i as f32 + 0.5) * step;
                let sy = y as f32 + (j as f32 + 0.5) * step;
                let (sr, sg, sb, sa) = self.sample(g, sx, sy);
                // Weight colour by alpha: a transparent sample has no colour
                // to contribute, and an edge pixel should take the colour of
                // whatever actually covers it.
                r += sr * sa;
                gg += sg * sa;
                b += sb * sa;
                a += sa;
            }
        }
        if a <= 0.0 {
            return (0.0, 0.0, 0.0, 0.0);
        }
        let n = (SUPERSAMPLE * SUPERSAMPLE) as f32;
        (r / a, gg / a, b / a, (a / n).clamp(0.0, 255.0))
    }
}

fn petal_angle(i: u8) -> f32 {
    (i as f32) * (2.0 * PI / PETAL_COUNT as f32)
}

/// Half-width of the petal strip at `t` (0 = disc edge, 1 = tip) as a share
/// of the full half-width: [`PETAL_PROFILE`] interpolated; the hidden base
/// under the disc keeps the disc-edge width.
fn petal_width_at(t: f32) -> f32 {
    if t >= 1.0 {
        return 0.0;
    }
    if t <= 0.0 {
        return PETAL_PROFILE[0];
    }
    let x = t * (PETAL_PROFILE.len() - 1) as f32;
    let i = x.floor() as usize;
    PETAL_PROFILE[i] + (PETAL_PROFILE[i + 1] - PETAL_PROFILE[i]) * (x - i as f32)
}

/// Returns (alpha, color) at point (px, py) for the petal at `frame`. Tip
/// colour blends white→pink along the outward half of the petal.
fn petal_sample(
    g: &Geometry,
    frame: &PetalFrame,
    alpha_scale: f32,
    px: f32,
    py: f32,
) -> (f32, [u8; 3]) {
    let PetalFrame {
        cx,
        cy,
        dir_x,
        dir_y,
    } = *frame;
    let perp_x = -dir_y;
    let perp_y = dir_x;

    let vx = px - cx;
    let vy = py - cy;
    let r_par = vx * dir_x + vy * dir_y;
    let r_per = vx * perp_x + vy * perp_y;

    // In or out of the strip: a hard edge, graded by the caller's
    // supersampling, like the Electron twin.
    let t = (r_par - g.center_radius) / (g.petal_tip - g.center_radius);
    let inside = r_par >= g.petal_base && r_per.abs() <= g.petal_width * petal_width_at(t);
    let a = if inside { alpha_scale } else { 0.0 };

    // Tip colouring: blend toward pink over the outward half. Squaring
    // concentrates the pink near the very tip.
    let tip_factor = ((t - 0.5) / 0.5).max(0.0).powi(2);
    let color = lerp_rgb(COLOR_PETAL_BASE, COLOR_PETAL_TIP, tip_factor);

    (a, color)
}

fn soft_disk(r: f32, radius: f32, ramp: f32) -> f32 {
    (radius - r + ramp).clamp(0.0, 1.0)
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
/// Returns straight colour and alpha, each clamped to `0..=255`.
fn composite(below: [u8; 3], a_below: f32, above: [u8; 3], a_above: f32) -> (f32, f32, f32, f32) {
    let a_out = a_above + a_below * (1.0 - a_above);
    if a_out <= 0.001 {
        return (0.0, 0.0, 0.0, 0.0);
    }
    let mix = |c_b: u8, c_a: u8| -> f32 {
        let v = (c_a as f32 * a_above + c_b as f32 * a_below * (1.0 - a_above)) / a_out;
        v.clamp(0.0, 255.0)
    };
    (
        mix(below[0], above[0]),
        mix(below[1], above[1]),
        mix(below[2], above[2]),
        (a_out * 255.0).clamp(0.0, 255.0),
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
        assert!(
            f.drift.is_none(),
            "5 s after a drop is past the drift window"
        );
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
        assert!(
            d.progress.abs() < 1e-6,
            "progress ≈ 0 at the instant of drop"
        );
        assert_eq!(f.time_since_drop, Duration::ZERO);
        assert!(!f.should_fire);
    }

    #[test]
    fn fires_only_after_final_drift_completes() {
        // drift_dur = 250 ms, so should_fire flips at duration + 250 ms.
        let just_before = frame_at(DUR_120S, DUR_120S + Duration::from_millis(200));
        assert!(
            !just_before.should_fire,
            "200 ms past expiry < 250 ms drift"
        );

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

    fn alpha_at(buf: &[u8], size: u32, x: u32, y: u32) -> u8 {
        let i = ((y * size + x) * 4 + 3) as usize;
        buf[i]
    }

    fn rgba_at(buf: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    // ---- render: pin the Windows bitmap byte-for-byte ----------------------
    //
    // `render` feeds the Windows notification-area icon. These hashes were
    // taken from its output before `render_at` existed; a change here means
    // the Windows daisy changed, which is never a side effect of macOS work.

    fn fnv1a64(bytes: &[u8]) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    const DRIFT_MID: DriftState = DriftState {
        petal_index: 5,
        progress: 0.5,
    };

    #[test]
    fn render_output_is_pinned() {
        let cases: [(u8, Option<DriftState>, u64); 3] = [
            (PETAL_COUNT, None, 0x649c_d681_fd7f_5185),
            (6, Some(DRIFT_MID), 0x6f98_fab0_5359_9cae),
            (0, None, 0x847a_9d62_4196_f5c5),
        ];
        for (petals, drift, expected) in cases {
            let buf = render(petals, drift);
            assert_eq!(buf.len(), (ICON_SIZE * ICON_SIZE * 4) as usize);
            let got = fnv1a64(&buf);
            assert_eq!(
                got, expected,
                "render({petals}, {drift:?}) changed: fnv1a64 = {got:#018x}"
            );
        }
    }

    #[test]
    fn full_daisy_has_top_petal_opaque() {
        let buf = render(PETAL_COUNT, None);
        // Top petal centered around y ≈ 31.5 - 20 = 11.5.
        // Vertical centerline at x = 32.
        assert!(
            alpha_at(&buf, ICON_SIZE, 32, 8) > 100,
            "top petal should be opaque"
        );
    }

    #[test]
    fn top_petal_tip_is_pink() {
        let buf = render(PETAL_COUNT, None);
        // Near the outer tip of the top petal (y ≈ 31.5 - PETAL_TIP ≈ 0.5)
        let i = ((3 * ICON_SIZE + (Geometry::windows().center as u32)) * 4) as usize;
        let r = buf[i];
        let g = buf[i + 1];
        let _b = buf[i + 2];
        let a = buf[i + 3];
        assert!(a > 60, "tip should be at least partially opaque");
        // Pink ≈ (240, 130, 165): R high, G clearly less than R, B between.
        assert!(r as i32 - g as i32 > 30, "tip should be warmer than white");
    }

    #[test]
    fn empty_daisy_has_only_center() {
        let buf = render(0, None);
        // Outer region — transparent.
        assert_eq!(alpha_at(&buf, ICON_SIZE, 32, 2), 0);
        // Dead center — opaque yellow.
        let c = Geometry::windows().center as u32;
        let i = ((c * ICON_SIZE + c) * 4) as usize;
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
            Some(DriftState {
                petal_index: 0,
                progress: 0.0,
            }),
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
            Some(DriftState {
                petal_index: 0,
                progress: 1.0,
            }),
        );
        assert!(alpha_at(&buf, ICON_SIZE, 32, 8) < 10);
    }

    // ---- render_at: one bitmap per host density --------------------------

    /// The sizes the macOS status item asks for: 18 px at 1x, 36 px at 2x.
    const HOST_SIZES: [u32; 2] = [18, 36];

    /// Pixel on the vertical centreline, through the top petal (petal 0,
    /// 12 o'clock) at `size`, three quarters of the way out: the body is
    /// still nearly full width there, and the neighbours' bases, which
    /// overlap the top petal's own out to r ≈ 18 of 64, are clear of it.
    fn top_petal_pixel(size: u32) -> (u32, u32) {
        let g = Geometry::at(size);
        (size / 2, (g.center - g.petal_tip * 0.75) as u32)
    }

    /// Spec §5: "Each petal is a strip that runs out from under the centre:
    /// narrowest where it leaves the disc, widest about three fifths of the
    /// way out, closed by a pointed-oval tip. Petal and centre meet with no
    /// gap between them."
    #[test]
    fn top_petal_meets_the_centre_with_no_gap() {
        for size in HOST_SIZES.into_iter().chain([ICON_SIZE]) {
            let g = Geometry::at(size);
            let buf = render_at(size, PETAL_COUNT, None);
            let x = size / 2;
            // Every pixel on the top petal's axis from the disc edge out to
            // the tip's cap is more than half covered by petal: no
            // transparent ring. (At 18 px the base is 1.2 px wide, so the
            // axis column is not fully covered there.)
            let first = (g.center - g.center_radius - 1.0) as u32;
            let last = (g.center - g.petal_tip + g.petal_width) as u32;
            for y in (last..=first).rev() {
                let [r, _, b, a] = rgba_at(&buf, size, x, y);
                assert!(a > 128, "size {size}: gap at ({x},{y}), alpha {a}");
                assert!(r > 200 && b > 100, "size {size}: not petal at ({x},{y})");
            }
        }
        // Narrower at the base than mid-length, closed at the tip: the
        // width profile itself, since next to the disc the neighbours'
        // bases overlap and no single pixel column belongs to one petal.
        assert!(petal_width_at(0.0) < petal_width_at(0.3));
        assert!(petal_width_at(0.3) < petal_width_at(0.6));
        assert!(petal_width_at(0.0) > 0.4);
        assert_eq!(petal_width_at(0.6), 1.0);
        assert!(petal_width_at(0.9) < 0.8);
        assert_eq!(petal_width_at(1.0), 0.0);
    }

    #[test]
    fn render_at_buffer_is_size_squared_rgba() {
        for size in HOST_SIZES {
            assert_eq!(
                render_at(size, PETAL_COUNT, None).len(),
                (size * size * 4) as usize
            );
        }
    }

    #[test]
    fn render_at_centre_pixel_is_opaque_yellow() {
        for size in HOST_SIZES {
            for petals in [PETAL_COUNT, 0] {
                let buf = render_at(size, petals, None);
                let [r, g, b, a] = rgba_at(&buf, size, size / 2, size / 2);
                assert!(a > 200, "size {size}, {petals} petals: centre alpha {a}");
                assert!(
                    r > 200 && g > 150 && b < 120,
                    "size {size}: not yellow ({r},{g},{b})"
                );
            }
        }
    }

    #[test]
    fn render_at_top_petal_present_when_attached_absent_when_not() {
        for size in HOST_SIZES {
            let (x, y) = top_petal_pixel(size);
            let full = render_at(size, PETAL_COUNT, None);
            assert!(
                alpha_at(&full, size, x, y) > 100,
                "size {size}: top petal missing at ({x},{y})"
            );
            let bare = render_at(size, 0, None);
            assert_eq!(
                alpha_at(&bare, size, x, y),
                0,
                "size {size}: something at ({x},{y}) with no petals"
            );
        }
    }

    /// At 18 px the top petal is about 2.6 px wide. A single sample per
    /// pixel leaves its cross-section as `0,255,255,0`; supersampling has to
    /// put graded alpha on the edge pixels instead.
    #[test]
    fn render_at_petal_edges_are_antialiased() {
        for size in HOST_SIZES {
            let g = Geometry::at(size);
            let (_, y) = top_petal_pixel(size);
            // Only the top petal's own columns: the 1 o'clock and 11 o'clock
            // petals reach into this row further out and would otherwise
            // supply graded pixels of their own.
            let span =
                (g.center - g.petal_width - 0.5) as u32..=(g.center + g.petal_width + 0.5) as u32;
            let buf = render_at(size, PETAL_COUNT, None);
            let row: Vec<u8> = span.map(|x| alpha_at(&buf, size, x, y)).collect();
            let intermediate = row.iter().filter(|&&a| a > 24 && a < 232).count();
            assert!(
                intermediate >= 2,
                "size {size}: no graded edge pixels across the top petal, row {y}: {row:?}"
            );
            let staircase = row
                .windows(2)
                .any(|w| w.iter().min().unwrap() < &24 && w.iter().max().unwrap() > &232);
            assert!(
                !staircase,
                "size {size}: hard 0→255 jump across the top petal, row {y}: {row:?}"
            );
        }
    }

    /// At progress 0 the drifting petal is exactly the attached one, at every
    /// size — the drift travel scales with the geometry.
    #[test]
    fn render_at_drift_at_progress_zero_matches_attached_petal() {
        for size in HOST_SIZES {
            let a = render_at(size, PETAL_COUNT, None);
            let b = render_at(
                size,
                PETAL_COUNT - 1,
                Some(DriftState {
                    petal_index: 0,
                    progress: 0.0,
                }),
            );
            assert_eq!(a, b, "size {size}");
        }
    }

    /// Writes `daisy-rust-18.png` and `daisy-rust-36.png` (full daisy) plus a
    /// mid-drift pair into `$NUDGE_DAISY_PNG_DIR` (default: the OS temp dir).
    /// Run with `--ignored` to eyeball the host-density bitmaps.
    #[test]
    #[ignore]
    fn png_dump_host_sizes() {
        let dir = std::env::var_os("NUDGE_DAISY_PNG_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        for size in HOST_SIZES {
            let save = |name: String, buf: Vec<u8>| {
                let path = dir.join(name);
                image::RgbaImage::from_raw(size, size, buf)
                    .expect("buffer is size*size*4")
                    .save(&path)
                    .expect("write png");
                println!("wrote {}", path.display());
            };
            save(
                format!("daisy-rust-{size}.png"),
                render_at(size, PETAL_COUNT, None),
            );
            save(
                format!("daisy-rust-{size}-drift.png"),
                render_at(
                    size,
                    PETAL_COUNT - 1,
                    Some(DriftState {
                        petal_index: 0,
                        progress: 0.5,
                    }),
                ),
            );
        }
    }

    /// Eyeball helper. Run with `--ignored --nocapture` to print all 13
    /// petal counts plus a mid-drift frame as ASCII.
    #[test]
    #[ignore]
    fn ascii_dump_all_states() {
        for petals in (0..=PETAL_COUNT).rev() {
            println!("\n=== {} petals remaining ===", petals);
            dump_ascii(&render(petals, None), ICON_SIZE);
        }
        println!("\n=== drift (petal 0, progress=0.5) ===");
        dump_ascii(
            &render(
                PETAL_COUNT - 1,
                Some(DriftState {
                    petal_index: 0,
                    progress: 0.5,
                }),
            ),
            ICON_SIZE,
        );
    }

    fn dump_ascii(buf: &[u8], size: u32) {
        for y in 0..size {
            let mut row = String::new();
            for x in 0..size {
                let i = ((y * size + x) * 4) as usize;
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
                    if a > 200 {
                        'P'
                    } else if a > 100 {
                        'p'
                    } else {
                        '.'
                    }
                } else {
                    if a > 200 {
                        '#'
                    } else if a > 100 {
                        '*'
                    } else {
                        '.'
                    }
                };
                row.push(ch);
            }
            println!("{}", row);
        }
    }
}
