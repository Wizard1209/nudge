//! Rasterise a candidate tray icon the way the app does, before it becomes
//! `assets/daisy.svg`: fallen petals spliced out by id (`petal-0` first),
//! the viewBox fitted into the square, resvg at the host's pixel size.
//!
//! ```sh
//! cargo run --example daisy_raster -- <out_dir> <sizes> <petals_remaining> <icon.svg>...
//! cargo run --example daisy_raster -- out 16,18,36 16,9 a.svg b.svg
//! ```
//!
//! Writes `<out_dir>/<stem>-<size>-<petals>.png` for every combination.
//! `docs/daisy/compare-colors.mjs` drives it; `src/daisy.rs` is the renderer
//! it mirrors and stays the one the app ships.

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn main() {
    eprintln!("daisy_raster needs resvg, a dependency of the Windows and macOS builds only");
}

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn main() {
    use resvg::{tiny_skia, usvg};
    use std::path::Path;

    let args: Vec<String> = std::env::args().skip(1).collect();
    let [out, sizes, petals, svgs @ ..] = args.as_slice() else {
        eprintln!("usage: daisy_raster <out_dir> <sizes> <petals_remaining> <icon.svg>...");
        std::process::exit(2);
    };
    let list = |s: &str| -> Vec<u32> {
        s.split(',')
            .map(|v| v.parse().expect("a comma-separated list of numbers"))
            .collect()
    };
    std::fs::create_dir_all(out).expect("the output directory");

    for path in svgs {
        let svg = std::fs::read_to_string(path).expect("a readable SVG");
        let stem = Path::new(path).file_stem().unwrap().to_string_lossy();
        for &remaining in &list(petals) {
            // Petals fall in index order: with N remaining, petal-0 … petal-(15-N) are gone.
            let mut frame = svg.clone();
            for k in 0..16u32.saturating_sub(remaining) {
                let open = format!("<g id=\"petal-{k}\"");
                let start = frame.find(&open).expect("every petal-k is in the SVG");
                let end = start + frame[start..].find("</g>").unwrap() + "</g>".len();
                frame.replace_range(start..end, "");
            }
            let tree = usvg::Tree::from_str(&frame, &usvg::Options::default()).expect("parses");
            let (w, h) = (tree.size().width(), tree.size().height());
            for &size in &list(sizes) {
                let mut pixmap = tiny_skia::Pixmap::new(size, size).unwrap();
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
                let file = Path::new(out).join(format!("{stem}-{size}-{remaining}.png"));
                pixmap.save_png(&file).expect("the PNG is written");
            }
        }
    }
}
