use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use resvg::{tiny_skia, usvg};

const SVG: &str = include_str!("../assets/daisy.svg");

pub const PETAL_COUNT: u8 = 16;

const DRIFT_OUTWARD: f32 = 8.0 / 64.0;
const DRIFT_GRAVITY: f32 = 6.0 / 64.0;

struct Daisy {
    petals: Vec<Range<usize>>,
    outward: Vec<(f32, f32)>,
    down: Vec<(f32, f32)>,
    canvas: (f32, f32),
}

static DAISY: LazyLock<Daisy> = LazyLock::new(|| {
    let petals = petal_ranges(SVG).unwrap_or_else(|e| panic!("assets/daisy.svg: {e}"));
    assert_eq!(
        petals.len(),
        PETAL_COUNT as usize,
        "assets/daisy.svg must hold petal-0 … petal-{}",
        PETAL_COUNT - 1
    );
    let tree = parse(SVG);
    let size = tree.size();
    let (cx, cy) = (size.width() / 2.0, size.height() / 2.0);
    let mut outward = Vec::with_capacity(petals.len());
    let mut down = Vec::with_capacity(petals.len());
    for k in 0..petals.len() {
        let node = tree
            .node_by_id(&format!("petal-{k}"))
            .unwrap_or_else(|| panic!("assets/daisy.svg: petal-{k} draws nothing"));
        let bbox = node.abs_bounding_box();
        let (dx, dy) = (
            bbox.x() + bbox.width() / 2.0 - cx,
            bbox.y() + bbox.height() / 2.0 - cy,
        );
        let len = dx.hypot(dy).max(f32::EPSILON);
        let to_parent = parent_transform(node).invert().unwrap_or_default();
        outward.push(map_vector(
            to_parent,
            dx / len * size.width(),
            dy / len * size.width(),
        ));
        down.push(map_vector(to_parent, 0.0, size.height()));
    }
    Daisy {
        petals,
        outward,
        down,
        canvas: (size.width(), size.height()),
    }
});

fn petal_ranges(svg: &str) -> Result<Vec<Range<usize>>, String> {
    let doc = roxmltree::Document::parse(svg).map_err(|e| e.to_string())?;
    let mut found: Vec<Option<Range<usize>>> = Vec::new();
    for node in doc.descendants().filter(|n| n.is_element()) {
        let Some(k) = node
            .attribute("id")
            .and_then(|id| id.strip_prefix("petal-"))
            .and_then(|k| k.parse::<usize>().ok())
        else {
            continue;
        };
        if found.len() <= k {
            found.resize(k + 1, None);
        }
        if found[k].replace(node.range()).is_some() {
            return Err(format!("petal-{k} appears twice"));
        }
    }
    found
        .into_iter()
        .enumerate()
        .map(|(k, r)| r.ok_or_else(|| format!("petal-{k} is missing")))
        .collect()
}

fn parse(svg: &str) -> usvg::Tree {
    usvg::Tree::from_str(svg, &usvg::Options::default()).expect("assets/daisy.svg parses")
}

fn parent_transform(node: &usvg::Node) -> tiny_skia::Transform {
    match node {
        usvg::Node::Group(g) => g
            .abs_transform()
            .pre_concat(g.transform().invert().unwrap_or_default()),
        other => other.abs_transform(),
    }
}

fn map_vector(t: tiny_skia::Transform, x: f32, y: f32) -> (f32, f32) {
    (t.sx * x + t.kx * y, t.ky * x + t.sy * y)
}

#[derive(Clone, Copy, Debug)]
pub struct DriftState {
    pub petal_index: u8,
    pub progress: f32,
}

#[derive(Clone, Copy, Debug)]
pub struct DaisyFrame {
    pub petals_remaining: u8,
    pub drift: Option<DriftState>,
    pub should_fire: bool,
    pub petal_duration: Duration,
    pub time_since_drop: Duration,
}

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

    let should_fire = elapsed >= duration + drift_dur;

    DaisyFrame {
        petals_remaining,
        drift,
        should_fire,
        petal_duration: petal_dur,
        time_since_drop,
    }
}

pub fn render(size: u32, petals_remaining: u8, drift: Option<DriftState>) -> Arc<[u8]> {
    if drift.is_some() {
        return rasterise(size, petals_remaining, drift).into();
    }
    type Frames = BTreeMap<(u32, u8), Arc<[u8]>>;
    static STEADY: Mutex<Frames> = Mutex::new(BTreeMap::new());
    let key = (size, petals_remaining.min(PETAL_COUNT));
    if let Some(hit) = STEADY.lock().unwrap().get(&key) {
        return hit.clone();
    }
    let frame: Arc<[u8]> = rasterise(size, key.1, None).into();
    STEADY.lock().unwrap().insert(key, frame.clone());
    frame
}

fn rasterise(size: u32, petals_remaining: u8, drift: Option<DriftState>) -> Vec<u8> {
    let tree = parse(&frame_svg(petals_remaining, drift));
    let mut pixmap = tiny_skia::Pixmap::new(size, size).expect("a non-zero icon size");
    let (w, h) = DAISY.canvas;
    let scale = (size as f32 / w).min(size as f32 / h);
    let fit = tiny_skia::Transform::from_row(
        scale,
        0.0,
        0.0,
        scale,
        (size as f32 - w * scale) / 2.0,
        (size as f32 - h * scale) / 2.0,
    );
    resvg::render(&tree, fit, &mut pixmap.as_mut());
    pixmap.take_demultiplied()
}

fn frame_svg(petals_remaining: u8, drift: Option<DriftState>) -> String {
    let daisy = &*DAISY;
    let fallen = PETAL_COUNT - petals_remaining.min(PETAL_COUNT);
    let falling = drift.filter(|d| d.petal_index < fallen);

    let mut gone: Vec<u8> = (0..fallen).collect();
    gone.sort_by_key(|&k| daisy.petals[k as usize].start);

    let mut out = String::with_capacity(SVG.len() + 96);
    let mut cursor = 0;
    for k in gone {
        let range = &daisy.petals[k as usize];
        if range.start < cursor {
            continue;
        }
        out.push_str(&SVG[cursor..range.start]);
        if let Some(d) = falling.filter(|d| d.petal_index == k) {
            let p = d.progress.clamp(0.0, 1.0);
            let (ox, oy) = daisy.outward[k as usize];
            let (gx, gy) = daisy.down[k as usize];
            let (out_k, sag_k) = (p * DRIFT_OUTWARD, p * p * DRIFT_GRAVITY);
            let (tx, ty) = (ox * out_k + gx * sag_k, oy * out_k + gy * sag_k);
            out.push_str(&format!(
                r#"<g transform="translate({tx} {ty})" opacity="{}">"#,
                1.0 - p
            ));
            out.push_str(&SVG[range.clone()]);
            out.push_str("</g>");
        }
        cursor = range.end;
    }
    out.push_str(&SVG[cursor..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const DUR_160S: Duration = Duration::from_secs(160);

    #[test]
    fn mid_interval_steady_state_has_no_drift() {
        let f = frame_at(DUR_160S, Duration::from_secs(35));
        assert_eq!(f.petals_remaining, 13);
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
        let f = frame_at(DUR_160S, Duration::from_secs(30));
        assert_eq!(f.petals_remaining, 13);
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
        let just_before = frame_at(DUR_160S, DUR_160S + Duration::from_millis(200));
        assert!(
            !just_before.should_fire,
            "200 ms past expiry < 250 ms drift"
        );

        let just_after = frame_at(DUR_160S, DUR_160S + Duration::from_millis(300));
        assert!(just_after.should_fire, "300 ms past expiry ≥ 250 ms drift");
        assert_eq!(just_after.petals_remaining, 0);

        let exact = frame_at(DUR_160S, DUR_160S + Duration::from_millis(250));
        assert!(exact.should_fire);
    }

    #[test]
    fn short_interval_clamps_drift_before_next_petal() {
        let dur = Duration::from_millis(1600);

        let inside = frame_at(dur, Duration::from_millis(120));
        assert_eq!(inside.petals_remaining, 15);
        let d = inside.drift.expect("20 ms after a drop is inside drift");
        assert_eq!(d.petal_index, 0);

        let after = frame_at(dur, Duration::from_millis(160));
        assert_eq!(after.petals_remaining, 15);
        assert!(
            after.drift.is_none(),
            "drift must clamp shut before the next petal drops"
        );
        assert_eq!(after.petal_duration, Duration::from_millis(100));
    }

    fn rgba_at(buf: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        [buf[i], buf[i + 1], buf[i + 2], buf[i + 3]]
    }

    fn fnv1a64(bytes: &[u8]) -> u64 {
        let mut h: u64 = 0xcbf2_9ce4_8422_2325;
        for &b in bytes {
            h ^= b as u64;
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
        h
    }

    fn petal_axes() -> Vec<(f32, f32)> {
        let doc = roxmltree::Document::parse(SVG).unwrap();
        (0..PETAL_COUNT)
            .map(|k| {
                let g = doc
                    .descendants()
                    .find(|n| n.attribute("id") == Some(&format!("petal-{k}")))
                    .unwrap();
                let rotate = g.attribute("transform").unwrap();
                let angle: f32 = rotate
                    .trim_start_matches("rotate(")
                    .split_whitespace()
                    .next()
                    .unwrap()
                    .parse()
                    .unwrap();
                let d = g
                    .descendants()
                    .find_map(|n| n.attribute("d"))
                    .expect("a petal path");
                let numbers: Vec<f32> = d
                    .split(|c: char| c.is_ascii_alphabetic() || c.is_whitespace() || c == ',')
                    .filter(|s| !s.is_empty())
                    .map(|s| s.parse().unwrap())
                    .collect();
                let top = numbers
                    .iter()
                    .skip(1)
                    .step_by(2)
                    .fold(f32::MAX, |a, &y| a.min(y));
                (angle, 32.0 - top)
            })
            .collect()
    }

    fn on_axis(size: u32, (angle, tip): (f32, f32), share: f32) -> (u32, u32) {
        let s = size as f32 / 64.0;
        let r = tip * share * s;
        let (sin, cos) = angle.to_radians().sin_cos();
        let c = size as f32 / 2.0;
        ((c + r * sin) as u32, (c - r * cos) as u32)
    }

    fn is_petal([_, _, b, a]: [u8; 4]) -> bool {
        a > 160 && b > 140
    }

    #[test]
    fn the_svg_holds_sixteen_petals_and_a_disc() {
        let doc = roxmltree::Document::parse(SVG).unwrap();
        let ids: Vec<&str> = doc
            .descendants()
            .filter_map(|n| n.attribute("id"))
            .collect();
        for k in 0..PETAL_COUNT {
            assert!(
                ids.contains(&format!("petal-{k}").as_str()),
                "petal-{k}: {ids:?}"
            );
        }
        assert!(ids.contains(&"disc"), "{ids:?}");
        assert_eq!(DAISY.petals.len(), PETAL_COUNT as usize);
    }

    #[test]
    fn petals_run_clockwise_from_twelve() {
        let axes = petal_axes();
        assert!(axes[0].0.abs() < 5.0, "petal-0 at {}°", axes[0].0);
        for w in axes.windows(2) {
            assert!(w[0].0 < w[1].0, "{axes:?}");
        }
    }

    #[test]
    fn petal_ranges_find_ids_at_any_depth() {
        let svg = r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 64 64"><g id="layer"><path id="petal-1" d="M0 0"/><g id="petal-0"><path d="M1 1"/></g></g></svg>"#;
        let ranges = petal_ranges(svg).unwrap();
        assert_eq!(
            &svg[ranges[0].clone()],
            r#"<g id="petal-0"><path d="M1 1"/></g>"#
        );
        assert_eq!(&svg[ranges[1].clone()], r#"<path id="petal-1" d="M0 0"/>"#);
        assert!(
            petal_ranges(r#"<svg><g id="petal-1"/></svg>"#).is_err(),
            "a gap"
        );
    }

    #[test]
    fn full_daisy_at_36_px_is_yellow_white_and_blue() {
        let size = 36;
        let buf = render(size, PETAL_COUNT, None);
        assert_eq!(buf.len(), (size * size * 4) as usize);
        let c = size / 2;
        let [r, g, b, a] = rgba_at(&buf, size, c, c);
        assert!(
            a == 255 && r > 240 && g > 180 && b < 80,
            "centre ({r},{g},{b},{a})"
        );

        let top = petal_axes()[0];
        let (x, y) = on_axis(size, top, 11.0 / top.1);
        let [r, g, b, a] = rgba_at(&buf, size, x, y);
        assert!(
            a > 200 && r > 230 && g > 230 && b > 230,
            "base ({r},{g},{b},{a}) at ({x},{y})"
        );
        let (x, y) = on_axis(size, top, 0.93);
        let [r, _, b, a] = rgba_at(&buf, size, x, y);
        assert!(
            a > 60 && b as i32 - r as i32 > 40,
            "tip ({r},{b},{a}) at ({x},{y})"
        );
    }

    #[test]
    fn each_fall_removes_exactly_its_petal() {
        let axes = petal_axes();
        for size in [18, 36] {
            for fallen in 0..=PETAL_COUNT {
                let buf = rasterise(size, PETAL_COUNT - fallen, None);
                let present: Vec<u8> = (0..PETAL_COUNT)
                    .filter(|&k| {
                        let (x, y) = on_axis(size, axes[k as usize], 0.75);
                        is_petal(rgba_at(&buf, size, x, y))
                    })
                    .collect();
                assert_eq!(
                    present,
                    (fallen..PETAL_COUNT).collect::<Vec<_>>(),
                    "size {size}, {fallen} fallen"
                );
            }
        }
    }

    #[test]
    fn bare_disc_has_nothing_outside_it() {
        let size = 36;
        let buf = render(size, 0, None);
        let opaque_outside = (0..size * size)
            .filter(|i| {
                let (x, y) = (i % size, i / size);
                let r = ((x as f32 + 0.5 - 18.0).powi(2) + (y as f32 + 0.5 - 18.0).powi(2)).sqrt();
                r > 8.5 * 36.0 / 64.0 + 1.0 && buf[(*i * 4 + 3) as usize] > 0
            })
            .count();
        assert_eq!(opaque_outside, 0);
    }

    #[test]
    fn renders_at_every_host_size() {
        for size in [16, 18, 20, 24, 32, 36, 48, 64] {
            let buf = render(size, PETAL_COUNT, None);
            assert_eq!(buf.len(), (size * size * 4) as usize, "size {size}");
            let c = size / 2;
            assert_eq!(rgba_at(&buf, size, c, c)[3], 255, "size {size}");
        }
    }

    #[test]
    fn steady_frames_come_from_the_cache() {
        let a = render(30, 7, None);
        let b = render(30, 7, None);
        assert!(Arc::ptr_eq(&a, &b));
    }

    #[test]
    fn render_output_is_pinned() {
        let cases: [(u8, u64); 2] = [
            (PETAL_COUNT, 0x8318_abba_657c_1610),
            (9, 0xd973_8681_fb22_92bd),
        ];
        for (petals, expected) in cases {
            let got = fnv1a64(&render(36, petals, None));
            assert_eq!(got, expected, "render(36, {petals}): fnv1a64 = {got:#018x}");
        }
    }

    #[test]
    fn drift_at_progress_zero_matches_attached_petal() {
        for size in [18, 36] {
            let a = render(size, PETAL_COUNT, None);
            let b = render(
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

    #[test]
    fn drift_moves_the_petal_outward_and_fades_it() {
        let size = 36;
        let top = petal_axes()[0];
        let mid = render(
            size,
            PETAL_COUNT - 1,
            Some(DriftState {
                petal_index: 0,
                progress: 0.5,
            }),
        );
        let (x, y) = on_axis(size, top, 0.75);
        let a = rgba_at(&mid, size, x, y)[3];
        assert!(a > 60 && a < 200, "half-faded petal, alpha {a}");
        let gone = render(
            size,
            PETAL_COUNT - 1,
            Some(DriftState {
                petal_index: 0,
                progress: 1.0,
            }),
        );
        assert_eq!(rgba_at(&gone, size, x, y)[3], 0, "faded out at progress 1");
    }

    #[test]
    #[ignore]
    fn perf_report() {
        use std::time::Instant;
        let t = Instant::now();
        let _ = &*DAISY;
        let svg_read = t.elapsed();
        let t = Instant::now();
        let _ = rasterise(36, PETAL_COUNT, None);
        let cold = t.elapsed();
        let _ = render(36, PETAL_COUNT, None);
        let n = 10_000;
        let t = Instant::now();
        for _ in 0..n {
            std::hint::black_box(render(36, PETAL_COUNT, None));
        }
        let warm = t.elapsed() / n;
        let t = Instant::now();
        for size in [18, 36] {
            for petals in 0..=PETAL_COUNT {
                std::hint::black_box(rasterise(size, petals, None));
            }
        }
        let all = t.elapsed();
        let t = Instant::now();
        std::hint::black_box(rasterise(
            36,
            PETAL_COUNT - 1,
            Some(DriftState {
                petal_index: 0,
                progress: 0.5,
            }),
        ));
        let drift = t.elapsed();
        println!(
            "first read of the SVG (ids, ranges, fall directions): {svg_read:?}\n\
             parse+render 36 px full frame: {cold:?}\n\
             cached fetch: {warm:?}\n\
             all 17 frames at 18 and 36 px: {all:?}\n\
             one drift frame at 36 px: {drift:?}"
        );
    }

    #[test]
    #[ignore]
    fn png_dump_host_sizes() {
        let dir = std::env::var_os("NUDGE_DAISY_PNG_DIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        for size in [18, 36] {
            let save = |name: String, buf: Arc<[u8]>| {
                let path = dir.join(name);
                image::RgbaImage::from_raw(size, size, buf.to_vec())
                    .expect("buffer is size*size*4")
                    .save(&path)
                    .expect("write png");
                println!("wrote {}", path.display());
            };
            save(format!("daisy-{size}.png"), render(size, PETAL_COUNT, None));
            save(format!("daisy-{size}-9.png"), render(size, 9, None));
            save(
                format!("daisy-{size}-drift.png"),
                render(
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
}
