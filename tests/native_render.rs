#![cfg(target_os = "windows")]

mod common;

use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC,
    DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDC, GetDIBits, GetMonitorInfoW, HGDIOBJ,
    MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint, MonitorFromWindow, ReleaseDC, SRCCOPY,
    SelectObject,
};
use windows::Win32::System::Threading::{OpenProcess, PROCESS_TERMINATE, TerminateProcess};
use windows::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, GetDpiForWindow, SetProcessDpiAwarenessContext,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
};
use windows::core::BOOL;

const STARTUP_WAIT: Duration = Duration::from_secs(11);

const CONFIG: &str = r#"{"hotkey":"Ctrl+Shift+Space","default_interval_minutes":0.05}"#;

const CARD_BRIGHTNESS_LO: u32 = 12;
const CARD_BRIGHTNESS_HI: u32 = 95;

const OPAQUE_DARKER_THAN_DESKTOP: f64 = 25.0;

const CARD_WIDTH_PT: f64 = 480.0;
const CARD_TOP_BELOW_QUARTER_PT: f64 = 80.0;

struct Shot {
    before: Capture,
    after: Capture,
    monitor: RECT,
    work: RECT,
    window: RECT,
    scale: f64,
}

fn spawn_and_capture() -> Shot {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }

    const ATTEMPTS: u32 = 4;
    for attempt in 1..=ATTEMPTS {
        match try_spawn_and_capture() {
            Some(shot) => return shot,
            None => eprintln!(
                "[native-render] attempt {attempt}/{ATTEMPTS}: popup hid before capture \
                 (focus-loss switch-away, spec §4) or never appeared — respawning"
            ),
        }
    }
    panic!(
        "could not capture nudge's popup in {ATTEMPTS} attempts — either it never rendered \
         (regression) or the desktop is busy enough to steal focus within ~400ms every time"
    );
}

fn try_spawn_and_capture() -> Option<Shot> {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_path = tmp.path().join("config.json");
    std::fs::write(&config_path, CONFIG).expect("write throwaway config");

    let (primary, _) = monitor_of(None);
    let before = Capture::of(primary);

    let exe = PathBuf::from(env!("CARGO_BIN_EXE_nudge"));
    let child = Command::new(&exe)
        .arg("--config")
        .arg(&config_path)
        .spawn()
        .expect("spawn nudge.exe");
    let mut guard = ChildGuard(child);
    let pid = guard.0.id();

    let poll_start = Instant::now();
    let mut seen = None;
    let (hwnd, window, monitor, work) = loop {
        if let Some((hwnd, rect)) = find_app_window(pid) {
            let first = *seen.get_or_insert((poll_start.elapsed(), rect));
            let (monitor, work) = monitor_of(Some(hwnd));
            if covers(rect, work) {
                if first.1 != rect {
                    eprintln!(
                        "[render-diag] window first seen at {:?} as {}x{}, placed over the work \
                         area {:?} after spawn",
                        first.0,
                        first.1.right - first.1.left,
                        first.1.bottom - first.1.top,
                        poll_start.elapsed()
                    );
                }
                break (hwnd, rect, monitor, work);
            }
            assert!(
                poll_start.elapsed() < STARTUP_WAIT,
                "the popup window is {:?}, the work area of its monitor {work:?}: spec §1 says the \
                 window covers the work area while visible",
                rect
            );
        } else if poll_start.elapsed() > STARTUP_WAIT {
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(
        (monitor.left, monitor.top, monitor.right, monitor.bottom),
        (primary.left, primary.top, primary.right, primary.bottom),
        "the popup opened on another monitor than the primary"
    );
    let scale = unsafe { GetDpiForWindow(hwnd) } as f64 / 96.0;

    std::thread::sleep(Duration::from_millis(400));
    let after = Capture::of(monitor);

    let survived = find_app_window(pid).is_some();

    let _ = guard.0.kill();
    let _ = guard.0.wait();

    survived.then_some(Shot {
        before,
        after,
        monitor,
        work,
        window,
        scale,
    })
}

fn covers(window: RECT, work: RECT) -> bool {
    const STRIP: i32 = 2;
    (window.left - work.left).abs() <= STRIP
        && (window.top - work.top).abs() <= STRIP
        && (window.right - work.right).abs() <= STRIP
        && (window.bottom - work.bottom).abs() <= STRIP
}

fn card_bbox(shot: &Shot) -> Option<(usize, usize, usize, usize, u64)> {
    let (mut min_x, mut min_y, mut max_x, mut max_y) = (usize::MAX, usize::MAX, 0usize, 0usize);
    let mut n = 0u64;
    let (ox, oy) = (shot.monitor.left, shot.monitor.top);
    let xs = (shot.window.left - ox).max(0) as usize
        ..((shot.window.right - ox) as usize).min(shot.after.w);
    let ys = (shot.window.top - oy).max(0) as usize
        ..((shot.window.bottom - oy) as usize).min(shot.after.h);
    for y in ys {
        for x in xs.clone() {
            let b = shot.after.brightness(x, y);
            let was = shot.before.brightness(x, y);
            if (CARD_BRIGHTNESS_LO..=CARD_BRIGHTNESS_HI).contains(&b) && b.abs_diff(was) > 10 {
                n += 1;
                min_x = min_x.min(x);
                min_y = min_y.min(y);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
    }
    (n > 0).then_some((min_x, min_y, max_x, max_y, n))
}

#[test]
#[ignore = "needs a real desktop+GPU, Windows-only, pops a window; run via --ignored"]
fn native_window_renders_card_centered_and_transparent() {
    let shot = spawn_and_capture();

    if let Ok(path) = std::env::var("NUDGE_DUMP_PNG") {
        shot.after.save_png(&path);
        eprintln!("[render-diag] wrote capture to {path}");
    }

    let (mw, mh) = (shot.after.w as f64, shot.after.h as f64);
    let s = shot.scale;
    let w = &shot.window;
    eprintln!(
        "[render-diag] monitor {mw}x{mh}, work area {:?}, window {}x{} at ({},{}), scale {s}",
        shot.work,
        w.right - w.left,
        w.bottom - w.top,
        w.left,
        w.top
    );

    let Some((min_x, min_y, max_x, max_y, card_px)) = card_bbox(&shot) else {
        panic!("found no card pixels on the monitor — popup not visible / not rendering?");
    };
    let card_w = (max_x - min_x + 1) as f64;
    let card_cx = (min_x + max_x) as f64 / 2.0;
    let want_top = mh * 0.25 + CARD_TOP_BELOW_QUARTER_PT * s;
    eprintln!(
        "[render-diag] card bbox=({min_x},{min_y})-({max_x},{max_y}), {card_px} px; \
         width {card_w} (want {:.0}), centre x {card_cx} (want {:.1}), top {min_y} (want {want_top:.0})",
        CARD_WIDTH_PT * s,
        mw / 2.0 - 0.5
    );
    assert!(
        card_px > (CARD_WIDTH_PT * s * 60.0 * s) as u64,
        "found almost no card pixels ({card_px}) — popup not visible / not rendering?"
    );

    assert!(
        (card_w - CARD_WIDTH_PT * s).abs() <= 0.06 * CARD_WIDTH_PT * s,
        "card is {card_w} px wide, expected about {:.0} ({CARD_WIDTH_PT} pt at scale {s}) — scale/layout regression?",
        CARD_WIDTH_PT * s
    );
    assert!(
        (card_cx - (mw / 2.0 - 0.5)).abs() <= 0.01 * mw,
        "card centre x {card_cx} is off the monitor's centre {:.1} — expected centred, not shoved into a corner",
        mw / 2.0 - 0.5
    );
    assert!(
        (min_y as f64 - want_top).abs() <= 8.0 * s,
        "card top {min_y} is not at 25% of the monitor height + {CARD_TOP_BELOW_QUARTER_PT} pt ({want_top:.0})"
    );

    const P: usize = 12;
    let (ox, oy) = (shot.monitor.left, shot.monitor.top);
    let (wx0, wy0) = ((w.left - ox) as usize, (w.top - oy) as usize);
    let (wx1, wy1) = ((w.right - ox) as usize, (w.bottom - oy) as usize);
    let spots = [
        (wx0 + 6, wy0 + 6),
        (wx1 - 6 - P, wy0 + 6),
        (wx0 + 6, wy1 - 6 - P),
        (wx1 - 6 - P, wy1 - 6 - P),
        ((wx0 + wx1) / 2, wy1 - 6 - P),
    ];
    let mut max_gap = f64::MIN;
    for (i, &(x, y)) in spots.iter().enumerate() {
        let now = shot.after.patch_mean(x, y, P);
        let was = shot.before.patch_mean(x, y, P);
        let gap = was - now;
        max_gap = max_gap.max(gap);
        eprintln!(
            "[render-diag] spot[{i}] with popup={now:.1} desktop before={was:.1} gap={gap:.1}"
        );
    }
    eprintln!(
        "[render-diag] worst spot opacity gap={max_gap:.1} (want < {OPAQUE_DARKER_THAN_DESKTOP})"
    );
    assert!(
        max_gap < OPAQUE_DARKER_THAN_DESKTOP,
        "a spot of the window is {max_gap:.1} darker than the desktop was there — the transparent overlay rendered as an opaque block (lost transparency)"
    );
}

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

#[test]
#[ignore = "needs a real desktop+GPU, Windows-only, pops a window, and an OPENAI_API_KEY + network; run via --ignored"]
fn native_window_passes_llm_vision_judge() {
    let shot = spawn_and_capture();
    let (x0, y0, x1, y1, _) = card_bbox(&shot).expect("no card pixels to judge");
    const MARGIN: usize = 40;
    let cap = shot.after.crop(
        x0.saturating_sub(MARGIN),
        y0.saturating_sub(MARGIN),
        (x1 + MARGIN).min(shot.after.w - 1),
        (y1 + MARGIN).min(shot.after.h - 1),
    );

    if let Ok(path) = std::env::var("NUDGE_DUMP_PNG") {
        cap.save_png(&path);
        eprintln!("[llm-judge] wrote judged image to {path}");
    }

    let scale: f32 = std::env::var("OPENAI_JUDGE_SCALE")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1.0);
    let Some((pass, reason)) = common::vision_verdict(&cap.png_bytes_scaled(scale), JUDGE_PROMPT)
    else {
        return;
    };
    assert!(pass, "LLM vision judge FAILED: {reason}");
}

struct Capture {
    px: Vec<u8>,
    w: usize,
    h: usize,
}

impl Capture {
    fn of(r: RECT) -> Capture {
        Capture::take(r.left, r.top, r.right - r.left, r.bottom - r.top)
    }

    fn crop(&self, x0: usize, y0: usize, x1: usize, y1: usize) -> Capture {
        let (w, h) = (x1 - x0 + 1, y1 - y0 + 1);
        let mut px = Vec::with_capacity(w * h * 4);
        for y in y0..=y1 {
            let row = (y * self.w + x0) * 4;
            px.extend_from_slice(&self.px[row..row + w * 4]);
        }
        Capture { px, w, h }
    }

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
                    biHeight: -h,
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

    fn rgba(&self) -> Vec<u8> {
        let mut rgba = vec![0u8; self.w * self.h * 4];
        for i in 0..(self.w * self.h) {
            rgba[i * 4] = self.px[i * 4 + 2];
            rgba[i * 4 + 1] = self.px[i * 4 + 1];
            rgba[i * 4 + 2] = self.px[i * 4];
            rgba[i * 4 + 3] = 255;
        }
        rgba
    }

    fn png_bytes_scaled(&self, scale: f32) -> Vec<u8> {
        let src = image::RgbaImage::from_raw(self.w as u32, self.h as u32, self.rgba())
            .expect("rgba buffer -> image");
        let (w, h, raw) = if (scale - 1.0).abs() > 1e-3 {
            let nw = (self.w as f32 * scale).round().max(1.0) as u32;
            let nh = (self.h as f32 * scale).round().max(1.0) as u32;
            let resized =
                image::imageops::resize(&src, nw, nh, image::imageops::FilterType::Lanczos3);
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
        let i = (y * self.w + x) * 4;
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
    found: Vec<(HWND, RECT, i32)>,
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
                if w > 100 && h > 100 {
                    data.found.push((hwnd, r, w * h));
                }
            }
        }
        BOOL(1)
    }
}

fn monitor_of(hwnd: Option<HWND>) -> (RECT, RECT) {
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe {
        let m = match hwnd {
            Some(h) => MonitorFromWindow(h, MONITOR_DEFAULTTOPRIMARY),
            None => MonitorFromPoint(Default::default(), MONITOR_DEFAULTTOPRIMARY),
        };
        assert!(
            GetMonitorInfoW(m, &mut info).as_bool(),
            "GetMonitorInfoW failed"
        );
    }
    (info.rcMonitor, info.rcWork)
}

fn find_app_window(pid: u32) -> Option<(HWND, RECT)> {
    let mut data = EnumData {
        pid,
        found: Vec::new(),
    };
    unsafe {
        let _ = EnumWindows(Some(enum_proc), LPARAM(&mut data as *mut _ as isize));
    }
    data.found
        .sort_by_key(|&(_, _, area)| std::cmp::Reverse(area));
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
