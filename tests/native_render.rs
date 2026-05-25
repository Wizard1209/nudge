//! Native window RENDERING regression test for nudge.exe.
//!
//! The eframe 0.31→0.34 upgrade fixed the idle-CPU busy-loop but regressed the
//! native window: the transparent borderless overlay came up as a *huge opaque
//! black rectangle* with the card *shrunk into the top-left corner* (the UI was
//! rendered at logical size into a larger physical surface — a DPI/scale bug —
//! and the surround cleared opaque instead of transparent). The WASM e2e tests
//! never caught it because they exercise the egui layer in a browser canvas,
//! which handles transparency and devicePixelRatio correctly — neither native
//! regression reproduces there.
//!
//! This test launches the real binary, locates its window, and SCREEN-captures
//! the window rect PLUS a margin of surrounding desktop via GDI
//! `BitBlt`+`GetDIBits`. Screen capture reads the DWM-composited output, so it
//! works for wgpu/GPU content (unlike `PrintWindow`, which returns black for
//! GPU surfaces). It then asserts:
//!
//!   1. The card is centered and fills most of the window (catches the
//!      top-left/scale regression). Needs the test process per-monitor-DPI-aware
//!      to read true physical pixels.
//!   2. The window is transparent: each corner *inside* the window matches the
//!      desktop *just outside* it. A correctly transparent overlay shows the
//!      same thing inside and outside its edge; the broken opaque-black window
//!      shows black inside while the desktop outside is unchanged. Comparing
//!      inside-vs-outside (rather than inside-vs-absolute-black) makes this
//!      independent of the wallpaper — even a solid black wallpaper won't
//!      false-fail it, because then inside and outside are both black and equal.
//!
//! Why `#[ignore]`: needs a real desktop/GPU, Windows-only, and pops a window.
//!
//! ```text
//! cargo test --release --target x86_64-pc-windows-gnu --test native_render -- --ignored --nocapture
//! ```

#![cfg(target_os = "windows")]

use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::Duration;

use base64::Engine as _;

use windows::core::BOOL;
use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, HGDIOBJ, ReleaseDC, SRCCOPY,
    SelectObject,
};
use windows::Win32::System::Threading::{OpenProcess, TerminateProcess, PROCESS_TERMINATE};
use windows::Win32::UI::HiDpi::{
    SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
};

/// Spawn → popup-visible settle. eframe init + tray bootstrap + foregrounding.
const STARTUP_WAIT: Duration = Duration::from_secs(4);

/// Desktop margin captured around the window, for the inside-vs-outside
/// transparency comparison. The window is screen-centered, so there's room.
const MARGIN: i32 = 24;

/// A pixel counts as "card" when its brightness is in this band: above the
/// opaque-black surround / transparent-over-dark (~0) and the card's own dark
/// fill (~24–46 depending on wallpaper bleed through the 86%-opaque fill), but
/// below bright wallpaper (>~100) and the light hint/label text (~170).
const CARD_BRIGHTNESS_LO: u32 = 12;
const CARD_BRIGHTNESS_HI: u32 = 95;

/// How much darker an inside-corner must be than the desktop just outside it
/// before we call the window opaque. Opaque-black-over-bright-wallpaper gives a
/// large gap; a transparent window (inside == outside) gives ~0.
const OPAQUE_DARKER_THAN_OUTSIDE: f64 = 25.0;

/// Spawn nudge.exe with a throwaway config, wait for its popup, screen-capture
/// the window plus `margin` px of surrounding desktop, kill the process, and
/// return `(capture, window_w, window_h)`. Sets the test process per-monitor
/// DPI-aware FIRST so GetWindowRect + the capture report true physical pixels
/// (without it everything is virtualized to logical coords, hiding scale bugs).
fn spawn_and_capture(margin: i32) -> (Capture, i32, i32) {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    let tmp = tempfile::tempdir().expect("tempdir");
    let config_path = tmp.path().join("config.json");
    std::fs::write(
        &config_path,
        r#"{"hotkey":"Ctrl+Shift+Space","default_interval_minutes":10.0}"#,
    )
    .expect("write throwaway config");

    let exe = PathBuf::from(env!("CARGO_BIN_EXE_nudge"));
    let child = Command::new(&exe)
        .arg("--config")
        .arg(&config_path)
        .spawn()
        .expect("spawn nudge.exe");
    let mut guard = ChildGuard(child);
    let pid = guard.0.id();

    // The popup is shown + foregrounded on startup and stays up (timer frozen
    // until first close), so we capture it without dismissing.
    std::thread::sleep(STARTUP_WAIT);

    let (_hwnd, rect) = find_app_window(pid)
        .expect("could not find nudge's visible top-level window — did it crash or stay hidden?");
    let w = (rect.right - rect.left).max(1);
    let h = (rect.bottom - rect.top).max(1);

    let cap = Capture::take(rect.left - margin, rect.top - margin, w + 2 * margin, h + 2 * margin);

    // Kill before the caller asserts so a failure never leaves nudge.exe behind.
    let _ = guard.0.kill();
    let _ = guard.0.wait();

    (cap, w, h)
}

#[test]
#[ignore = "needs a real desktop+GPU, Windows-only, pops a window; run via --ignored"]
fn native_window_renders_card_centered_and_transparent() {
    let (cap, w, h) = spawn_and_capture(MARGIN);

    // Debug: dump the raw capture so we can eyeball what the brightness-band
    // heuristic is actually measuring. Set NUDGE_DUMP_PNG to a path.
    if let Ok(path) = std::env::var("NUDGE_DUMP_PNG") {
        cap.save_png(&path);
        eprintln!("[render-diag] wrote capture to {path}");
    }

    // The window occupies [MARGIN, MARGIN+w) x [MARGIN, MARGIN+h) in `cap`.
    let win_x0 = MARGIN as usize;
    let win_y0 = MARGIN as usize;
    let (wu, hu) = (w as usize, h as usize);

    // --- locate the card: bounding box of in-band pixels within the window ---
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (wu, hu, 0usize, 0usize);
    let mut card_px = 0u64;
    for y in 0..hu {
        for x in 0..wu {
            let b = cap.brightness(win_x0 + x, win_y0 + y);
            if (CARD_BRIGHTNESS_LO..=CARD_BRIGHTNESS_HI).contains(&b) {
                card_px += 1;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    assert!(
        card_px > (wu * hu / 200) as u64,
        "found almost no card pixels ({card_px}) in the {w}x{h} window — popup not visible / not rendering?"
    );

    let card_w = (max_x - min_x + 1) as f64;
    let card_cx = (min_x + max_x) as f64 / 2.0;
    let card_cy = (min_y + max_y) as f64 / 2.0;
    let width_frac = card_w / w as f64;
    let cx_off = (card_cx - w as f64 / 2.0).abs() / w as f64;
    let cy_off = (card_cy - h as f64 / 2.0).abs() / h as f64;

    // --- transparency: inside-corner vs just-outside-the-window ---
    // For each corner, sample a patch just inside the window edge and a patch in
    // the desktop margin just outside it. Transparent → inside ≈ outside.
    // Opaque-black-over-wallpaper → inside much darker than outside.
    const P: usize = 12; // patch size
    let inside = [
        (win_x0 + 6, win_y0 + 6),                         // TL
        (win_x0 + wu - 6 - P, win_y0 + 6),                // TR
        (win_x0 + 6, win_y0 + hu - 6 - P),                // BL
        (win_x0 + wu - 6 - P, win_y0 + hu - 6 - P),       // BR
    ];
    let outside = [
        (2, 2),
        (win_x0 + wu + 2, 2),
        (2, win_y0 + hu + 2),
        (win_x0 + wu + 2, win_y0 + hu + 2),
    ];
    let mut max_gap = f64::MIN; // most-opaque corner (outside brighter than inside)
    for i in 0..4 {
        let inside_mean = cap.patch_mean(inside[i].0, inside[i].1, P);
        let outside_mean = cap.patch_mean(outside[i].0, outside[i].1, P);
        let gap = outside_mean - inside_mean;
        max_gap = max_gap.max(gap);
        eprintln!(
            "[render-diag] corner[{i}] inside={inside_mean:.1} outside={outside_mean:.1} gap={gap:.1}"
        );
    }

    eprintln!("[render-diag] window {w}x{h}, card bbox=({min_x},{min_y})-({max_x},{max_y})");
    eprintln!(
        "[render-diag] card width_frac={width_frac:.2} (want >=0.55), center off x={cx_off:.2} y={cy_off:.2} (want small)"
    );
    eprintln!("[render-diag] worst corner opacity gap={max_gap:.1} (want < {OPAQUE_DARKER_THAN_OUTSIDE})");

    // 1. Card fills + centers (catches the top-left/scale regression).
    assert!(
        width_frac >= 0.55,
        "card spans only {width_frac:.2} of window width — expected it to fill most of the window (scale/layout regression?)"
    );
    assert!(
        cx_off < 0.12 && cy_off < 0.18,
        "card center is off ({cx_off:.2},{cy_off:.2} of window) — expected centered, not shoved into a corner"
    );

    // 2. Transparent surround (catches the opaque-black window), wallpaper-independent.
    assert!(
        max_gap < OPAQUE_DARKER_THAN_OUTSIDE,
        "a window corner is {max_gap:.1} darker than the desktop just outside it — the transparent overlay rendered as an opaque block (lost transparency)"
    );
}

/// What the LLM vision judge must affirm about the popup. Targets the class of
/// bug the brightness heuristic above is blind to: the field labels rendering
/// jammed in the top-left CORNER of each field (lost text inset) rather than
/// padded + vertically centered. The judge sees the real composited pixels.
const JUDGE_PROMPT: &str = "\
You are a UI QA judge checking for ONE specific rendering bug. The image is a \
screenshot of a minimalist 'spotlight'-style popup card for a journaling app. \
The card is TRANSPARENT outside its rounded rectangle, so the desktop (chat \
windows, etc.) shows through around and faintly behind it — that is EXPECTED; \
never fail because of background content. Judge ONLY the rounded card, which \
has three stacked rows: a field labelled 'Что я делаю?', a field labelled \
'Хуйня?', and a number field (e.g. '10').\n\n\
THE BUG TO CATCH: a field's text rendered jammed into the TOP-LEFT CORNER of \
its row — flush against the left and top edges with no padding.\n\n\
PASS when each row's text is clearly INDENTED from the card's left edge (has \
left padding) and sits roughly in the row's vertical middle. Treat minor \
variation in exact vertical centering, padding size, font smoothing, or \
faint background bleed-through as NORMAL — do NOT fail for any of those.\n\n\
FAIL ONLY if: text is jammed flush in the top-left corner with no padding, OR \
a whole row/label is missing, OR a label is so cut off it is unreadable.\n\n\
Respond with ONLY a JSON object, no prose: \
{\"pass\": true|false, \"reason\": \"<one concise sentence>\"}.";

/// LLM-as-judge: capture the popup and ask an OpenAI vision model whether it
/// renders correctly. Catches visual regressions (text-in-corner, clipping,
/// missing fields) that the pixel-brightness heuristics can't distinguish from
/// a transparent window over a busy/dark desktop. Skips (passes) when
/// OPENAI_API_KEY is unset so it never blocks CI without credentials.
#[test]
#[ignore = "needs a real desktop+GPU, Windows-only, pops a window, and an OPENAI_API_KEY + network; run via --ignored"]
fn native_window_passes_llm_vision_judge() {
    let api_key = match std::env::var("OPENAI_API_KEY") {
        Ok(k) if !k.trim().is_empty() => k,
        _ => {
            eprintln!("[llm-judge] OPENAI_API_KEY unset — skipping (no-op pass)");
            return;
        }
    };

    // No margin: hand the judge just the card window itself.
    let (cap, _w, _h) = spawn_and_capture(0);

    // Dump exactly what the judge sees, when NUDGE_DUMP_PNG is set.
    if let Ok(path) = std::env::var("NUDGE_DUMP_PNG") {
        cap.save_png(&path);
        eprintln!("[llm-judge] wrote judged image to {path}");
    }

    // Default gpt-4o: gpt-4o-mini's vision can't resolve the ~20px text inset
    // on this transparent card over a busy desktop — it false-fails a CORRECT
    // render at full/half/low resolution alike (a capability ceiling, not a
    // cost knob). Override with OPENAI_JUDGE_MODEL=gpt-4o-mini at your peril.
    let model = std::env::var("OPENAI_JUDGE_MODEL").unwrap_or_else(|_| "gpt-4o".to_string());
    // Cost knobs: OPENAI_JUDGE_DETAIL "low" (default) sends one flat ~512px /
    // ~85-token tile — gpt-4o still judges correctly and it's ~13× cheaper than
    // "high". OPENAI_JUDGE_SCALE downscales the PNG before encoding (1.0 = full).
    let scale: f32 = std::env::var("OPENAI_JUDGE_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    let detail = std::env::var("OPENAI_JUDGE_DETAIL").unwrap_or_else(|_| "low".to_string());
    let png = cap.png_bytes_scaled(scale);
    eprintln!(
        "[llm-judge] model={model} scale={scale} detail={detail} png_bytes={}",
        png.len()
    );
    let data_url = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&png)
    );
    let request = serde_json::json!({
        "model": model,
        "temperature": 0,
        "max_tokens": 200,
        "response_format": { "type": "json_object" },
        "messages": [{
            "role": "user",
            "content": [
                { "type": "text", "text": JUDGE_PROMPT },
                { "type": "image_url", "image_url": { "url": data_url, "detail": detail } }
            ]
        }]
    });

    let response = ureq::post("https://api.openai.com/v1/chat/completions")
        .set("Authorization", &format!("Bearer {api_key}"))
        .send_json(request)
        .expect("OpenAI request failed");
    let body: serde_json::Value = response.into_json().expect("parse OpenAI response");

    let content = body["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or_else(|| panic!("no content in OpenAI response: {body}"));
    eprintln!("[llm-judge] verdict: {content}");

    let verdict: serde_json::Value =
        serde_json::from_str(content).expect("judge did not return JSON");
    let pass = verdict["pass"].as_bool().unwrap_or(false);
    let reason = verdict["reason"].as_str().unwrap_or("(no reason given)");
    assert!(pass, "LLM vision judge FAILED: {reason}");
}

/// A captured BGRA top-down image of a screen rect.
struct Capture {
    px: Vec<u8>,
    w: usize,
    h: usize,
}

impl Capture {
    fn take(left: i32, top: i32, w: i32, h: i32) -> Capture {
        let (wu, hu) = (w.max(1) as usize, h.max(1) as usize);
        let mut px = vec![0u8; wu * hu * 4];
        unsafe {
            let screen = GetDC(None);
            let mem = CreateCompatibleDC(Some(screen));
            let bmp = CreateCompatibleBitmap(screen, w, h);
            let old = SelectObject(mem, HGDIOBJ(bmp.0));
            let _ = BitBlt(mem, 0, 0, w, h, Some(screen), left, top, SRCCOPY);
            let mut bmi = BITMAPINFO {
                bmiHeader: BITMAPINFOHEADER {
                    biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                    biWidth: w,
                    biHeight: -h, // negative = top-down
                    biPlanes: 1,
                    biBitCount: 32,
                    biCompression: BI_RGB.0,
                    ..Default::default()
                },
                ..Default::default()
            };
            let scanned = GetDIBits(
                mem,
                bmp,
                0,
                h as u32,
                Some(px.as_mut_ptr() as *mut _),
                &mut bmi,
                DIB_RGB_COLORS,
            );
            assert!(scanned != 0, "GetDIBits failed to read the captured bitmap");
            SelectObject(mem, old);
            let _ = DeleteObject(HGDIOBJ(bmp.0));
            let _ = DeleteDC(mem);
            ReleaseDC(None, screen);
        }
        Capture { px, w: wu, h: hu }
    }

    /// Captured BGRA → RGBA bytes (swizzle B/R, force opaque alpha so the
    /// image is viewable/encodable regardless of the captured alpha channel).
    fn rgba(&self) -> Vec<u8> {
        let mut rgba = vec![0u8; self.w * self.h * 4];
        for i in 0..(self.w * self.h) {
            rgba[i * 4] = self.px[i * 4 + 2]; // R
            rgba[i * 4 + 1] = self.px[i * 4 + 1]; // G
            rgba[i * 4 + 2] = self.px[i * 4]; // B
            rgba[i * 4 + 3] = 255; // A
        }
        rgba
    }

    /// Encode the capture as an in-memory PNG (for the LLM vision judge),
    /// optionally downscaled by `scale` (1.0 = full resolution).
    fn png_bytes_scaled(&self, scale: f32) -> Vec<u8> {
        let src = image::RgbaImage::from_raw(self.w as u32, self.h as u32, self.rgba())
            .expect("rgba buffer -> image");
        let (w, h, raw) = if (scale - 1.0).abs() > 1e-3 {
            let nw = (self.w as f32 * scale).round().max(1.0) as u32;
            let nh = (self.h as f32 * scale).round().max(1.0) as u32;
            let resized = image::imageops::resize(&src, nw, nh, image::imageops::FilterType::Lanczos3);
            (nw, nh, resized.into_raw())
        } else {
            (self.w as u32, self.h as u32, src.into_raw())
        };
        let mut out = std::io::Cursor::new(Vec::new());
        image::write_buffer_with_format(
            &mut out,
            &raw,
            w,
            h,
            image::ExtendedColorType::Rgba8,
            image::ImageFormat::Png,
        )
        .expect("encode png");
        out.into_inner()
    }

    fn png_bytes(&self) -> Vec<u8> {
        self.png_bytes_scaled(1.0)
    }

    fn save_png(&self, path: &str) {
        std::fs::write(path, self.png_bytes()).expect("write png");
    }

    fn brightness(&self, x: usize, y: usize) -> u32 {
        if x >= self.w || y >= self.h {
            return 0;
        }
        let i = (y * self.w + x) * 4; // BGRA
        (self.px[i] as u32 + self.px[i + 1] as u32 + self.px[i + 2] as u32) / 3
    }

    fn patch_mean(&self, x0: usize, y0: usize, size: usize) -> f64 {
        let mut sum = 0u64;
        let mut n = 0u64;
        for y in y0..(y0 + size).min(self.h) {
            for x in x0..(x0 + size).min(self.w) {
                sum += self.brightness(x, y) as u64;
                n += 1;
            }
        }
        if n == 0 { 0.0 } else { sum as f64 / n as f64 }
    }
}

struct EnumData {
    pid: u32,
    found: Vec<(HWND, RECT, i32)>, // hwnd, rect, area
}

extern "system" fn enum_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    unsafe {
        let data = &mut *(lparam.0 as *mut EnumData);
        let mut wpid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut wpid));
        if wpid == data.pid && IsWindowVisible(hwnd).as_bool() {
            let mut r = RECT::default();
            if GetWindowRect(hwnd, &mut r).is_ok() {
                let w = r.right - r.left;
                let h = r.bottom - r.top;
                // Skip the tray's hidden message window and other tiny helpers.
                if w > 100 && h > 100 {
                    data.found.push((hwnd, r, w * h));
                }
            }
        }
        BOOL(1) // keep enumerating
    }
}

/// The largest visible top-level window owned by `pid` — the eframe popup.
fn find_app_window(pid: u32) -> Option<(HWND, RECT)> {
    let mut data = EnumData { pid, found: Vec::new() };
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut data as *mut _ as isize));
    }
    data.found.sort_by_key(|&(_, _, area)| std::cmp::Reverse(area));
    data.found.first().map(|&(hwnd, rect, _)| (hwnd, rect))
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let pid = self.0.id();
        let _ = self.0.kill();
        let _ = self.0.wait();
        unsafe {
            if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, pid) {
                let _ = TerminateProcess(h, 1);
                let _ = windows::Win32::Foundation::CloseHandle(h);
            }
        }
    }
}
