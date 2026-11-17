#![cfg(target_os = "macos")]

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use core_graphics::display::{CGDisplay, kCGWindowListOptionAll};
use core_graphics::event::{CGEventFlags, CGKeyCode};
use core_graphics::geometry::CGRect as CGWindowRect;
use core_graphics::window::{
    kCGWindowBounds, kCGWindowIsOnscreen, kCGWindowLayer, kCGWindowName, kCGWindowNumber,
    kCGWindowOwnerPID,
};

const STARTUP_GRACE: Duration = Duration::from_secs(5);

struct SandboxedApp {
    child: Child,
    home: tempfile::TempDir,
}

const CONFIG_SUBDIR: &str = "Library/Application Support/Nudge";

impl Drop for SandboxedApp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

const SPRITE_GREEN: &[u8] = include_bytes!("../electron/tests/e2e/fixtures/sprite-green.png");
const SPRITE_FILE: &str = "sprite.png";

impl SandboxedApp {
    fn launch() -> Self {
        let home = tempfile::tempdir().expect("temp HOME");
        std::fs::create_dir_all(home.path().join("Documents")).expect("Documents");

        let child = Command::new(binary_path())
            .env("HOME", home.path())
            .spawn()
            .expect("spawn nudge — run `cargo build --bin nudge` first");

        std::thread::sleep(STARTUP_GRACE);
        let app = Self { child, home };
        app.summon();
        app
    }

    fn launch_with_interval(minutes: f64) -> Self {
        Self::launch_inner(
            &format!(
                r#"{{"hotkey": "Ctrl+Shift+Space", "default_interval_minutes": {minutes}, "autostart": false}}"#
            ),
            false,
        )
    }

    fn launch_with_config(config: &str) -> Self {
        let app = Self::launch_inner(config, false);
        app.summon();
        app
    }

    fn launch_with_config_logging(config: &str) -> Self {
        let app = Self::launch_inner(config, true);
        app.summon();
        app
    }

    fn summon(&self) {
        assert!(
            !popup_visible(self.pid()),
            "the popup is on screen after launch — spec §4 starts in Waiting"
        );
        press_chord(
            VK_SPACE,
            CGEventFlags::CGEventFlagControl | CGEventFlags::CGEventFlagShift,
        );
        assert!(
            wait_until(Duration::from_secs(3), || popup_visible(self.pid())),
            "the hotkey did not open the popup — harness problem (PostEvent grant?), \
             not what the test is about"
        );
    }

    fn launch_settings() -> Self {
        let home = tempfile::tempdir().expect("temp HOME");
        std::fs::create_dir_all(home.path().join("Documents")).expect("Documents");

        let child = Command::new(binary_path())
            .arg("--settings")
            .env("HOME", home.path())
            .spawn()
            .expect("spawn nudge — run `cargo build --bin nudge` first");

        std::thread::sleep(STARTUP_GRACE);
        Self { child, home }
    }

    fn launch_inner(config: &str, capture_stderr: bool) -> Self {
        let home = tempfile::tempdir().expect("temp HOME");
        std::fs::create_dir_all(home.path().join("Documents")).expect("Documents");
        let sprite = home.path().join(SPRITE_FILE);
        std::fs::write(&sprite, SPRITE_GREEN).expect("write the sprite fixture");
        let config_dir = home.path().join(CONFIG_SUBDIR);
        std::fs::create_dir_all(&config_dir).expect("config dir");
        std::fs::write(
            config_dir.join("config.json"),
            config.replace("{sprite}", &sprite.display().to_string()),
        )
        .expect("seed config.json");

        let mut cmd = Command::new(binary_path());
        cmd.env("HOME", home.path());
        if capture_stderr {
            cmd.stderr(Stdio::piped());
        }
        let child = cmd
            .spawn()
            .expect("spawn nudge — run `cargo build --bin nudge` first");

        std::thread::sleep(STARTUP_GRACE);
        Self { child, home }
    }

    fn stderr_after_exit(&mut self) -> String {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let mut text = String::new();
        if let Some(mut pipe) = self.child.stderr.take() {
            use std::io::Read;
            let _ = pipe.read_to_string(&mut text);
        }
        text
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn sprite_path(&self) -> PathBuf {
        self.home.path().join(SPRITE_FILE)
    }

    fn config_path(&self) -> PathBuf {
        self.home.path().join(CONFIG_SUBDIR).join("config.json")
    }

    fn journal_path(&self) -> PathBuf {
        self.home.path().join("Documents/Nudge/journal-rust.ndjson")
    }
}

fn binary_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/nudge")
}

fn config_with_sprite(enabled: bool, x: i32, y: i32, card_width: u32) -> String {
    format!(
        r#"{{"hotkey": "Ctrl+Shift+Space", "default_interval_minutes": 10, "autostart": false,
  "sprite": {{"enabled": {enabled}, "path": "{{sprite}}", "x": {x}, "y": {y}, "card_width": {card_width}}}}}"#
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Rect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

impl Rect {
    fn intersects(&self, other: &Rect) -> bool {
        self.x < other.x + other.w
            && other.x < self.x + self.w
            && self.y < other.y + other.h
            && other.y < self.y + self.h
    }

    fn same_as(&self, other: &Rect) -> bool {
        (self.x - other.x).abs() <= 1.0
            && (self.y - other.y).abs() <= 1.0
            && (self.w - other.w).abs() <= 1.0
            && (self.h - other.h).abs() <= 1.0
    }
}

#[derive(Debug, Clone, Copy)]
struct WindowInfo {
    id: u32,
    rect: Rect,
    on_screen: bool,
}

fn window_rect(pid: u32) -> Option<WindowInfo> {
    let windows = CGDisplay::window_list_info(kCGWindowListOptionAll, None)?;

    let owner_pid_key = unsafe { CFString::wrap_under_get_rule(kCGWindowOwnerPID) };
    let bounds_key = unsafe { CFString::wrap_under_get_rule(kCGWindowBounds) };
    let number_key = unsafe { CFString::wrap_under_get_rule(kCGWindowNumber) };
    let on_screen_key = unsafe { CFString::wrap_under_get_rule(kCGWindowIsOnscreen) };

    let layer_key = unsafe { CFString::wrap_under_get_rule(kCGWindowLayer) };

    let mut best: Option<WindowInfo> = None;

    for i in 0..windows.len() {
        let Some(entry) = windows.get(i) else {
            continue;
        };
        let dict: CFDictionary<CFString, CFType> =
            unsafe { TCFType::wrap_under_get_rule(*entry as CFDictionaryRef) };

        let owner_pid = dict
            .find(&owner_pid_key)
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|n| n.to_i64());
        if owner_pid != Some(pid as i64) {
            continue;
        }

        let Some(bounds_dict) = dict
            .find(&bounds_key)
            .and_then(|v| v.downcast::<CFDictionary>())
        else {
            continue;
        };
        let Some(cg_rect) = CGWindowRect::from_dict_representation(&bounds_dict) else {
            continue;
        };
        if cg_rect.size.width <= 0.0 || cg_rect.size.height <= 0.0 {
            continue;
        }

        let id = dict
            .find(&number_key)
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|n| n.to_i64())
            .unwrap_or(0) as u32;
        let rect = Rect {
            x: cg_rect.origin.x,
            y: cg_rect.origin.y,
            w: cg_rect.size.width,
            h: cg_rect.size.height,
        };
        let on_screen = dict
            .find(&on_screen_key)
            .and_then(|v| v.downcast::<CFBoolean>())
            .map(bool::from)
            .unwrap_or(false);

        let layer = dict
            .find(&layer_key)
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|n| n.to_i64())
            .unwrap_or(0);
        if !(1..STATUS_ITEM_MIN_LAYER).contains(&layer) {
            continue;
        }
        let candidate = WindowInfo {
            id,
            rect,
            on_screen,
        };
        if best.is_none_or(|b| rect.w * rect.h > b.rect.w * b.rect.h) {
            best = Some(candidate);
        }
    }

    best
}

fn popup_visible(pid: u32) -> bool {
    window_rect(pid).is_some_and(|w| w.on_screen)
}

fn popup_stays_visible(pid: u32) -> bool {
    popup_visible(pid) && {
        std::thread::sleep(Duration::from_millis(250));
        popup_visible(pid)
    }
}

fn dismiss_with_escape() {
    let focused = Command::new("osascript")
        .arg("-e")
        .arg(r#"tell application "System Events" to set frontmost of process "nudge" to true"#)
        .status()
        .expect("osascript not runnable");
    assert!(
        focused.success(),
        "osascript failed to focus the nudge process — is a GUI session active?"
    );

    let pressed = Command::new("osascript")
        .arg("-e")
        .arg(r#"tell application "System Events" to key code 53"#)
        .status()
        .expect("osascript not runnable");
    assert!(
        pressed.success(),
        "osascript failed to send Esc — is Accessibility granted to the terminal running cargo?"
    );
}

fn main_display_frame() -> Rect {
    let bounds = CGDisplay::main().bounds();
    Rect {
        x: bounds.origin.x,
        y: bounds.origin.y,
        w: bounds.size.width,
        h: bounds.size.height,
    }
}

fn main_screen_facts() -> (Rect, f64) {
    let out = Command::new("osascript")
        .args([
            "-l",
            "JavaScript",
            "-e",
            r#"ObjC.import("AppKit"); const s = $.NSScreen.mainScreen; const f = s.visibleFrame; [f.origin.x, f.origin.y, f.size.width, f.size.height, s.backingScaleFactor].join(" ")"#,
        ])
        .output()
        .expect("osascript not runnable");
    assert!(
        out.status.success(),
        "osascript could not read NSScreen.mainScreen: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: Vec<f64> = String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .map(|n| n.parse().expect("NSScreen facts are numbers"))
        .collect();
    assert_eq!(
        v.len(),
        5,
        "visibleFrame plus the scale is five numbers, got {v:?}"
    );
    (
        Rect {
            x: v[0],
            y: v[1],
            w: v[2],
            h: v[3],
        },
        v[4],
    )
}

fn work_area() -> Rect {
    let (visible, _) = main_screen_facts();
    let display = main_display_frame();
    Rect {
        x: visible.x,
        y: display.y + display.h - (visible.y + visible.h),
        w: visible.w,
        h: visible.h,
    }
}

fn display_scale() -> f64 {
    main_screen_facts().1
}

fn sprite_points() -> (f64, f64) {
    let scale = display_scale();
    (720.0 / scale, 430.0 / scale)
}

fn sprite_fixture() -> (image::RgbaImage, (u32, u32, u32, u32)) {
    let img = image::load_from_memory(SPRITE_GREEN)
        .expect("the embedded fixture decodes")
        .to_rgba8();
    let bbox = bbox_of(&img, is_green).expect("the fixture has opaque pixels");
    (img, bbox)
}

fn sprite_opaque_top() -> u32 {
    sprite_fixture().1.1
}

fn card_anchor_top(display: &Rect) -> f64 {
    display.y + (display.h / 4.0).floor() + 80.0
}

fn bbox_rect(window: &WindowInfo, scale: f64, (x0, y0, x1, y1): (u32, u32, u32, u32)) -> Rect {
    Rect {
        x: window.rect.x + f64::from(x0) / scale,
        y: window.rect.y + f64::from(y0) / scale,
        w: f64::from(x1 - x0 + 1) / scale,
        h: f64::from(y1 - y0 + 1) / scale,
    }
}

fn capture_window(id: u32, dest: &Path) -> image::RgbaImage {
    let status = Command::new("screencapture")
        .arg("-x")
        .arg("-o")
        .arg("-l")
        .arg(id.to_string())
        .arg(dest)
        .status()
        .expect("screencapture not runnable");

    assert!(
        status.success() && dest.exists(),
        "screencapture -l produced nothing — grant Screen Recording to the terminal \
         running cargo (System Settings › Privacy & Security › Screen Recording), \
         then restart it so the running process picks the grant up"
    );

    image::open(dest)
        .expect("captured file is not a decodable image")
        .to_rgba8()
}

fn is_green(p: &image::Rgba<u8>) -> bool {
    let [r, g, b, a] = p.0;
    a > 128 && g >= 200 && r <= 80 && b <= 80
}

fn bbox_of(
    img: &image::RgbaImage,
    keep: impl Fn(&image::Rgba<u8>) -> bool,
) -> Option<(u32, u32, u32, u32)> {
    let mut found: Option<(u32, u32, u32, u32)> = None;
    for (x, y, p) in img.enumerate_pixels() {
        if !keep(p) {
            continue;
        }
        found = Some(match found {
            None => (x, y, x, y),
            Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x), y1.max(y)),
        });
    }
    found
}

fn card_bbox(img: &image::RgbaImage) -> (u32, u32, u32, u32) {
    let (w, h) = img.dimensions();
    bbox_of(img, |p| p.0[3] > 128 && !is_green(p)).unwrap_or_else(|| {
        panic!("no painted pixels in the {w}x{h} window capture — the popup rendered nothing")
    })
}

fn sprite_bbox(img: &image::RgbaImage) -> Option<(u32, u32, u32, u32)> {
    bbox_of(img, is_green)
}

fn luma_over_black(img: &image::RgbaImage, x: u32, y: u32) -> f64 {
    let [r, g, b, a] = img.get_pixel(x, y).0;
    let a = f64::from(a) / 255.0;
    a * (0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b))
}

fn capture_region(rect: Rect, dest: &Path) {
    let status = Command::new("screencapture")
        .arg("-x")
        .arg("-R")
        .arg(format!("{},{},{},{}", rect.x, rect.y, rect.w, rect.h))
        .arg(dest)
        .status()
        .expect("screencapture not runnable");

    assert!(
        status.success() && dest.exists(),
        "screencapture produced nothing — grant Screen Recording to the terminal \
         running cargo (System Settings › Privacy & Security › Screen Recording), \
         then restart it so the running process picks the grant up"
    );
}

#[test]
#[ignore = "needs a GUI session + Screen Recording grant; run via --ignored"]
fn harness_can_sandbox_and_capture() {
    let app = SandboxedApp::launch();

    assert!(
        app.config_path().exists(),
        "app did not write {} — it never got far enough to be worth testing",
        app.config_path().display()
    );
    assert!(
        !app.home.path().join("Documents/Nudge/config.json").exists(),
        "config must not be created under Documents — that folder is TCC-gated \
         and iCloud-synced (design-spec §9)"
    );

    let shot = app.home.path().join("display.png");
    capture_region(main_display_frame(), &shot);

    let img = image::open(&shot).expect("captured file is not a decodable image");
    assert!(
        img.width() > 0 && img.height() > 0,
        "captured an empty image"
    );
}

#[test]
#[ignore = "needs a GUI session; run via --ignored"]
fn a_config_left_in_documents_is_carried_over_to_application_support() {
    let home = tempfile::tempdir().expect("temp HOME");
    let legacy = home.path().join("Documents/Nudge/config.json");
    std::fs::create_dir_all(legacy.parent().unwrap()).expect("Documents/Nudge");
    std::fs::write(
        &legacy,
        r#"{"hotkey": "Ctrl+Shift+Space", "default_interval_minutes": 37.5, "autostart": false}"#,
    )
    .expect("seed the legacy config");

    let child = Command::new(binary_path())
        .env("HOME", home.path())
        .spawn()
        .expect("spawn nudge — run `cargo build --bin nudge` first");
    let mut app = SandboxedApp { child, home };
    std::thread::sleep(STARTUP_GRACE);

    let moved = std::fs::read_to_string(app.config_path())
        .unwrap_or_else(|e| panic!("no config at {}: {e}", app.config_path().display()));
    assert!(
        moved.contains("37.5"),
        "the destination holds a fresh default, not the user's file: {moved}"
    );
    assert!(
        !legacy.exists(),
        "the old file survived — two configs now diverge silently"
    );
    let _ = app.child.kill();
}

#[test]
#[ignore = "needs a GUI session + Screen Recording grant; run via --ignored"]
fn window_covers_the_work_area_and_the_card_sits_at_its_anchor() {
    let app = SandboxedApp::launch();
    let window = window_rect(app.pid()).expect("no nudge window found");
    let display = main_display_frame();
    let work = work_area();
    eprintln!("window={:?} display={display:?} work={work:?}", window.rect);

    assert!(
        window.rect.intersects(&display),
        "popup is off-screen at {:?} — invisible to the user despite a live process",
        window.rect
    );
    assert!(
        window.rect.same_as(&work),
        "the window is {:?}, not the work area {work:?} — spec §1: while visible it \
         covers the monitor's work area, so a click anywhere beside the card closes \
         the popup and reaches nothing",
        window.rect
    );

    let (window, img) = capture_card(&app);
    let card = bbox_rect(&window, capture_scale(&img, &window), card_bbox(&img));
    eprintln!("card={card:?}");
    let centre = card.x + card.w / 2.0;
    let want_centre = display.x + display.w / 2.0;
    assert!(
        (centre - want_centre).abs() <= 1.0,
        "the card's centre is at x={centre}, the display's at x={want_centre} — spec §1 \
         centres the card on the monitor"
    );
    let want_top = card_anchor_top(&display);
    let pixels_top = display.y + (display.h * display_scale() / 4.0).floor() + 80.0;
    assert!(
        (card.y - want_top).abs() <= 1.0,
        "spec §1 wants the card's top at 25% of the {}x{}-point display plus 80 pt, \
         at y={want_top}; it is at y={}. Feeding the same arithmetic this display's \
         *pixels* instead would put it at y={pixels_top} — which is the mistake this \
         assertion exists to catch, so check the units before adjusting the expectation.",
        display.w,
        display.h,
        card.y
    );
}

#[test]
#[ignore = "needs a GUI session + Accessibility grant (System Events synthesises the Esc \
            keystroke); run via --ignored"]
fn popup_reappears_after_the_interval() {
    const INTERVAL: Duration = Duration::from_secs(3);
    const INTERVALS_TO_WAIT: u32 = 4;

    let app = SandboxedApp::launch_with_interval(0.05);

    assert!(
        wait_until(INTERVAL * 2, || popup_visible(app.pid())),
        "the first deadline never showed the popup — the interval timer does not \
         fire from launch"
    );
    std::thread::sleep(Duration::from_millis(500));

    dismiss_with_escape();
    std::thread::sleep(Duration::from_millis(500));
    assert!(
        !popup_visible(app.pid()),
        "Esc did not dismiss the popup — synthetic input is not reaching the app"
    );

    let poll_interval = Duration::from_millis(500);
    let deadline = std::time::Instant::now() + INTERVAL * INTERVALS_TO_WAIT;
    let mut reappeared = false;
    while std::time::Instant::now() < deadline {
        if popup_visible(app.pid()) {
            reappeared = true;
            break;
        }
        std::thread::sleep(poll_interval);
    }

    assert!(
        reappeared,
        "popup did not reappear within {:?} (~{INTERVALS_TO_WAIT} intervals) — the interval \
         timer never fires natively on macOS: `Timer::is_expired` is compiled only under \
         cfg(any(test, wasm32)), and the sole native expiry path lives in the Windows-only \
         tray thread",
        INTERVAL * INTERVALS_TO_WAIT
    );
}

const STATUS_ITEM_MIN_LAYER: i64 = 20;

#[derive(Debug, Clone, Copy, PartialEq)]
struct MenuBarWindow {
    id: u32,
    owner_pid: u32,
    layer: i64,
    rect: Rect,
}

fn menu_bar_windows() -> Vec<MenuBarWindow> {
    let Some(windows) = CGDisplay::window_list_info(kCGWindowListOptionAll, None) else {
        return Vec::new();
    };

    let owner_pid_key = unsafe { CFString::wrap_under_get_rule(kCGWindowOwnerPID) };
    let bounds_key = unsafe { CFString::wrap_under_get_rule(kCGWindowBounds) };
    let number_key = unsafe { CFString::wrap_under_get_rule(kCGWindowNumber) };
    let on_screen_key = unsafe { CFString::wrap_under_get_rule(kCGWindowIsOnscreen) };
    let layer_key = unsafe { CFString::wrap_under_get_rule(kCGWindowLayer) };

    let mut found = Vec::new();
    for i in 0..windows.len() {
        let Some(entry) = windows.get(i) else {
            continue;
        };
        let dict: CFDictionary<CFString, CFType> =
            unsafe { TCFType::wrap_under_get_rule(*entry as CFDictionaryRef) };

        let layer = dict
            .find(&layer_key)
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|n| n.to_i64())
            .unwrap_or(0);
        if layer < STATUS_ITEM_MIN_LAYER {
            continue;
        }
        let on_screen = dict
            .find(&on_screen_key)
            .and_then(|v| v.downcast::<CFBoolean>())
            .map(bool::from)
            .unwrap_or(false);
        if !on_screen {
            continue;
        }
        let Some(cg_rect) = dict
            .find(&bounds_key)
            .and_then(|v| v.downcast::<CFDictionary>())
            .and_then(|b| CGWindowRect::from_dict_representation(&b))
        else {
            continue;
        };

        found.push(MenuBarWindow {
            id: dict
                .find(&number_key)
                .and_then(|v| v.downcast::<CFNumber>())
                .and_then(|n| n.to_i64())
                .unwrap_or(0) as u32,
            owner_pid: dict
                .find(&owner_pid_key)
                .and_then(|v| v.downcast::<CFNumber>())
                .and_then(|n| n.to_i64())
                .unwrap_or(0) as u32,
            layer,
            rect: Rect {
                x: cg_rect.origin.x,
                y: cg_rect.origin.y,
                w: cg_rect.size.width,
                h: cg_rect.size.height,
            },
        });
    }
    found
}

#[test]
#[ignore = "needs a GUI session; run via --ignored"]
fn menu_bar_status_item_is_present() {
    let before = menu_bar_windows();
    assert!(
        !before.is_empty(),
        "no menu-bar windows visible at all — the window server is not showing a menu bar to \
         this session (a locked screen does this), so the difference below would be unreadable"
    );

    let app = SandboxedApp::launch();
    let during = menu_bar_windows();

    let added: Vec<MenuBarWindow> = during
        .iter()
        .filter(|w| !before.iter().any(|b| b.id == w.id))
        .copied()
        .collect();
    eprintln!(
        "[status-item] nudge pid {}; menu-bar windows {} before → {} during; added {added:#?}",
        app.pid(),
        before.len(),
        during.len(),
    );
    assert!(
        !added.is_empty(),
        "launching nudge added no window to the menu bar\n  before: {before:#?}\n  during: {during:#?}"
    );
    let status_items: Vec<MenuBarWindow> = added
        .iter()
        .filter(|w| w.rect.y >= 0.0 && w.rect.y < 40.0)
        .copied()
        .collect();
    assert!(
        !status_items.is_empty(),
        "nudge added {} menu-bar window(s), none of them a status item sitting in the strip at \
         the top of the screen: {added:#?}",
        added.len()
    );

    drop(app);
    std::thread::sleep(Duration::from_secs(2));
    let after = menu_bar_windows();

    let survivors: Vec<&MenuBarWindow> = status_items
        .iter()
        .filter(|w| after.iter().any(|a| a.id == w.id))
        .collect();
    assert!(
        survivors.is_empty(),
        "the window(s) nudge added to the menu bar outlived the process, so they were never \
         nudge's: {survivors:#?}"
    );
}

fn status_item_frames() -> Vec<(f64, f64, f64, f64)> {
    let out = Command::new("osascript")
        .arg("-e")
        .arg(
            r#"tell application "System Events" to tell process "nudge" to get {position, size} of menu bar items of menu bar 2"#,
        )
        .output()
        .expect("osascript not runnable");
    assert!(
        out.status.success(),
        "osascript could not read the menu bar extras — is Accessibility granted to the \
         terminal running cargo?\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let numbers: Vec<f64> = String::from_utf8_lossy(&out.stdout)
        .split(',')
        .filter_map(|t| t.trim().parse().ok())
        .collect();
    let n = numbers.len() / 4;
    (0..n)
        .map(|i| {
            (
                numbers[2 * i],
                numbers[2 * i + 1],
                numbers[2 * n + 2 * i],
                numbers[2 * n + 2 * i + 1],
            )
        })
        .collect()
}

#[test]
#[ignore = "needs a GUI session; run via --ignored"]
fn status_item_fits_the_menu_bar() {
    let _app = SandboxedApp::launch();
    let items = status_item_frames();
    eprintln!("[status-item] frames {items:?}");
    assert_eq!(
        items.len(),
        1,
        "expected exactly one status item, got {items:?}"
    );
    let (_, y, w, h) = items[0];
    assert!(
        y >= 0.0,
        "the status item starts above the menu bar: {items:?}"
    );
    assert!(
        h <= 24.0,
        "the status item is taller than a menu bar: {items:?}"
    );
    assert!(
        w <= 48.0,
        "the status item is wider than a menu bar icon: {items:?}"
    );
}

fn status_item_menu_labels() -> Option<Vec<String>> {
    let out = Command::new("osascript")
        .arg("-e")
        .arg(
            r#"tell application "System Events" to tell process "nudge"
                if exists menu 1 of menu bar item 1 of menu bar 2 then
                    get name of menu items of menu 1 of menu bar item 1 of menu bar 2
                else
                    return "<none>"
                end if
            end tell"#,
        )
        .output()
        .expect("osascript not runnable");
    assert!(
        out.status.success(),
        "osascript could not read the status item's menu:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if text == "<none>" {
        return None;
    }
    Some(text.split(", ").map(str::to_string).collect())
}

fn open_menu_window(pid: u32) -> Option<Rect> {
    menu_bar_windows()
        .into_iter()
        .filter(|w| w.owner_pid == pid)
        .map(|w| w.rect)
        .find(|r| r.y >= 20.0 && r.h > 24.0)
}

fn status_item_centre() -> (f64, f64) {
    let items = status_item_frames();
    assert_eq!(
        items.len(),
        1,
        "expected exactly one status item, got {items:?}"
    );
    let (x, y, w, h) = items[0];
    (x + w / 2.0, y + h / 2.0)
}

#[test]
#[ignore = "needs a GUI session + PostEvent grant; run via --ignored"]
fn left_clicking_the_status_item_opens_the_popup() {
    let app = SandboxedApp::launch();
    assert!(
        popup_visible(app.pid()),
        "the popup never appeared — harness problem, not a status-item problem"
    );
    press_escape();
    assert!(
        wait_until(Duration::from_secs(3), || !popup_visible(app.pid())),
        "Esc did not dismiss the popup — synthetic input is not reaching the app"
    );

    let centre = status_item_centre();
    eprintln!("[status-item] left click at {centre:?}");
    click_at(centre);
    assert!(
        wait_until(Duration::from_secs(3), || popup_visible(app.pid())),
        "a left click on the status item did not bring the popup back"
    );

    std::thread::sleep(Duration::from_millis(500));
    dismiss_with_escape();
    assert!(
        wait_until(Duration::from_secs(3), || !popup_visible(app.pid())),
        "the popup the click opened did not take Esc"
    );
}

#[test]
#[ignore = "needs a GUI session + PostEvent grant; run via --ignored"]
fn right_clicking_the_status_item_shows_the_menu() {
    let app = SandboxedApp::launch();
    assert!(
        popup_visible(app.pid()),
        "the popup never appeared — harness problem, not a status-item problem"
    );
    press_escape();
    assert!(
        wait_until(Duration::from_secs(3), || !popup_visible(app.pid())),
        "Esc did not dismiss the popup — synthetic input is not reaching the app"
    );

    assert!(
        open_menu_window(app.pid()).is_none(),
        "a menu is already up before anything was clicked"
    );
    let centre = status_item_centre();
    eprintln!("[status-item] right click at {centre:?}");
    right_click_at(centre);

    let mut menu = None;
    assert!(
        wait_until(Duration::from_secs(3), || {
            menu = open_menu_window(app.pid());
            menu.is_some()
        }),
        "a right click on the status item did not open its menu"
    );
    let labels = status_item_menu_labels();
    eprintln!("[status-item] menu window {menu:?}, items {labels:?}");
    let expected: Vec<String> = nudge_web::nudge_state::TRAY_MENU_LABELS
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        labels.as_ref(),
        Some(&expected),
        "the menu is not the spec's three items"
    );
    assert!(
        !popup_visible(app.pid()),
        "a right click must show the menu, not open the popup"
    );

    press_escape();
    assert!(
        wait_until(Duration::from_secs(3), || open_menu_window(app.pid())
            .is_none()),
        "Esc did not close the status item's menu"
    );

    let centre = status_item_centre();
    eprintln!("[status-item] left click after the menu closed, at {centre:?}");
    click_at(centre);
    assert!(
        wait_until(Duration::from_secs(3), || popup_visible(app.pid())),
        "after the menu closed, a left click on the status item no longer opens the popup — \
         the menu stayed attached to the item"
    );

    std::thread::sleep(Duration::from_millis(500));
    dismiss_with_escape();
    assert!(
        wait_until(Duration::from_secs(3), || !popup_visible(app.pid())),
        "the popup the click opened did not take Esc"
    );
}

fn status_item_menu_row_frame(label: &str) -> (f64, f64, f64, f64) {
    let script = format!(
        r#"tell application "System Events" to tell process "nudge" to get {{position, size}} of menu item "{label}" of menu 1 of menu bar item 1 of menu bar 2"#
    );
    let out = Command::new("osascript")
        .arg("-e")
        .arg(&script)
        .output()
        .expect("osascript not runnable");
    assert!(
        out.status.success(),
        "osascript could not read the frame of menu row {label:?}:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let numbers: Vec<f64> = String::from_utf8_lossy(&out.stdout)
        .split(',')
        .filter_map(|t| t.trim().parse().ok())
        .collect();
    let [x, y, w, h] = numbers[..] else {
        panic!(
            "expected x, y, w, h for menu row {label:?}, got {:?}",
            String::from_utf8_lossy(&out.stdout)
        );
    };
    (x, y, w, h)
}

#[test]
#[ignore = "needs a GUI session + PostEvent grant; run via --ignored"]
fn choosing_show_nudge_from_the_status_item_menu_opens_the_popup() {
    let app = SandboxedApp::launch();
    assert!(
        popup_visible(app.pid()),
        "the popup never appeared — harness problem, not a status-item problem"
    );
    press_escape();
    assert!(
        wait_until(Duration::from_secs(3), || !popup_visible(app.pid())),
        "Esc did not dismiss the popup — synthetic input is not reaching the app"
    );

    let centre = status_item_centre();
    eprintln!("[status-item] right click at {centre:?}");
    right_click_at(centre);
    let mut menu = None;
    assert!(
        wait_until(Duration::from_secs(3), || {
            menu = open_menu_window(app.pid());
            menu.is_some()
        }),
        "a right click on the status item did not open its menu"
    );

    let show_label = nudge_web::nudge_state::TRAY_MENU_LABELS[0];
    let (x, y, w, h) = status_item_menu_row_frame(show_label);
    let row_centre = (x + w / 2.0, y + h / 2.0);
    eprintln!(
        "[status-item] menu window {menu:?}, row {show_label:?} at ({x}, {y}, {w}, {h}), \
         clicking {row_centre:?}"
    );
    let menu = menu.expect("the menu rect was just observed");
    assert!(
        x >= menu.x
            && y >= menu.y
            && x + w <= menu.x + menu.w + 1.0
            && y + h <= menu.y + menu.h + 1.0,
        "the row {show_label:?} at ({x}, {y}, {w}, {h}) lies outside the menu window {menu:?}"
    );
    click_at(row_centre);

    assert!(
        wait_until(Duration::from_secs(3), || popup_visible(app.pid())),
        "choosing {show_label:?} from the status item's menu did not open the popup"
    );
    assert!(
        open_menu_window(app.pid()).is_none(),
        "the menu is still up after one of its rows was chosen"
    );

    std::thread::sleep(Duration::from_millis(500));
    dismiss_with_escape();
    assert!(
        wait_until(Duration::from_secs(3), || !popup_visible(app.pid())),
        "the popup the menu opened did not take Esc"
    );
}

#[test]
#[ignore = "needs a GUI session + PostEvent grant; run via --ignored"]
fn submitting_appends_one_journal_line() {
    const TYPED: &str = "cgevent submit probe";

    let app = SandboxedApp::launch();
    assert!(
        popup_visible(app.pid()),
        "the popup never appeared — nothing to type into, so a red here is a \
         harness problem rather than a submit-path problem"
    );
    assert!(
        !app.journal_path().exists(),
        "the sandbox HOME already has a journal at {} before anything was typed",
        app.journal_path().display()
    );

    type_text(TYPED);
    press_return();

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while popup_visible(app.pid()) && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(200));
    }
    assert!(
        !popup_visible(app.pid()),
        "Return did not close the popup — no keystroke reached the app at all, so the \
         PostEvent grant or the GUI session is the thing to check first"
    );

    let journal = std::fs::read_to_string(app.journal_path()).unwrap_or_else(|e| {
        panic!(
            "no journal at {}: {e} — the popup closed, so Return arrived, but the \
             characters before it did not: the app took the key codes without ever \
             becoming key (`decide_close` writes nothing when every text field is empty)",
            app.journal_path().display()
        )
    });

    let lines: Vec<&str> = journal.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(
        lines.len(),
        1,
        "one submit must append exactly one NDJSON line, got {}:\n{journal}",
        lines.len()
    );

    let entry: serde_json::Value =
        serde_json::from_str(lines[0]).expect("journal line is not valid JSON");
    assert_eq!(entry["event_type"], "submitted");
    assert_eq!(entry["implementation"], "rust");
    assert_eq!(
        entry["doing"], TYPED,
        "the line was written but without the typed text, so Return reached the app \
         and the characters did not"
    );
}

fn capture_scale(img: &image::RgbaImage, window: &WindowInfo) -> f64 {
    f64::from(img.width()) / window.rect.w
}

fn capture_card(app: &SandboxedApp) -> (WindowInfo, image::RgbaImage) {
    let window = window_rect(app.pid()).expect("no nudge window found");
    assert!(
        popup_visible(app.pid()),
        "nudge has a window but it is not on screen, so there is nothing to \
         photograph — check that no other nudge is running"
    );
    let img = capture_window(window.id, &app.home.path().join("card.png"));
    (window, img)
}

#[test]
#[ignore = "needs a GUI session + Screen Recording grant; run via --ignored"]
fn card_contour_survives_a_dark_backdrop() {
    let app = SandboxedApp::launch();
    let (window, img) = capture_card(&app);
    let scale = capture_scale(&img, &window);

    let (x0, y0, x1, y1) = card_bbox(&img);
    let pt = |v: f64| (v * scale).round() as u32;
    let rim_px = pt(1.0).max(1);
    let fill_probe = rim_px + pt(3.0);
    let (from, to) = (y0 + pt(24.0), y1 - pt(24.0));
    assert!(from < to, "card too short to sample: {y0}..{y1}");

    let mut weakest = f64::MAX;
    let mut failures = 0;
    for y in from..to {
        for (edge, inward) in [(x0, 1i64), (x1, -1i64)] {
            let at = |off: u32| {
                let x = (i64::from(edge) + inward * i64::from(off)) as u32;
                luma_over_black(&img, x, y)
            };
            let rim = (0..rim_px).map(at).fold(f64::MIN, f64::max);
            let gap = rim - at(fill_probe);
            weakest = weakest.min(gap);
            if gap < 15.0 {
                failures += 1;
            }
        }
    }

    let sampled = (to - from) * 2;
    assert!(
        failures * 10 < sampled,
        "the card has no contour along {failures} of {sampled} sampled edge points \
         (weakest rim-over-fill gap {weakest:.1}, need 15). Composited over black \
         the shadow adds nothing, so without the hairline the card has no boundary \
         at all — see CARD_STROKE in src/app.rs and spec §2."
    );
}

#[test]
#[ignore = "needs a GUI session + Screen Recording grant; run via --ignored"]
fn focused_row_tint_is_a_field_inside_the_row() {
    let app = SandboxedApp::launch();
    let (window, img) = capture_card(&app);
    let scale = capture_scale(&img, &window);

    let (x0, y0, x1, _) = card_bbox(&img);
    let pt = |v: f64| (v * scale).round() as u32;
    let focused_y = y0 + pt(20.0);
    let plain_y = y0 + pt(61.0);
    let middle_x = (x0 + x1) / 2;
    let gutter_x = x0 + pt(1.0).max(1) + pt(2.0);
    let field_x = x0 + pt(14.0);
    let delta = |x: u32, y: u32| luma_over_black(&img, x, y) - luma_over_black(&img, x, plain_y);

    let at_middle = delta(middle_x, focused_y);
    assert!(
        at_middle >= 8.0,
        "the focused row is not tinted at all (Δ{at_middle:.1} against the row below, \
         need 8) — focus is invisible, so the edge measurements below would be \
         meaningless"
    );

    let at_field_edge = delta(field_x, focused_y);
    let at_gutter = delta(gutter_x, focused_y);
    assert!(
        at_field_edge >= 8.0 && at_gutter < 3.0,
        "the focused row is tinted in its middle (Δ{at_middle:.1}) but sideways the tint \
         ends in the wrong place: Δ{at_field_edge:.1} at 14pt in (need 8), Δ{at_gutter:.1} \
         at 3pt in (need under 3). The field is inset 10pt from the row's sides — see \
         FOCUS_FIELD_INSET in src/app.rs and spec §3."
    );

    let scan_from = focused_y;
    let scan_to = y0 + pt(70.0);
    let (divider_y, _) = (scan_from..scan_to)
        .map(|y| (y, luma_over_black(&img, middle_x, y)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .expect("empty scan band");
    let above_divider = delta(middle_x, divider_y - pt(2.0).max(1));
    let in_field = delta(middle_x, divider_y - pt(9.0));
    assert!(
        in_field >= 8.0 && above_divider < 3.0,
        "the focused row is tinted at its middle (Δ{at_middle:.1}) but downwards the tint \
         ends in the wrong place: Δ{in_field:.1} 9pt above the divider (need 8), \
         Δ{above_divider:.1} 2pt above it (need under 3). The field is inset 5pt from \
         the band the row really consumed — `row_field` measures that band from the \
         cursor, because the field's own margins make the row taller than it allocates."
    );
}

struct Backdrop(Child);

impl Drop for Backdrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Backdrop {
    fn up(pattern: &str) -> Self {
        let exe =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/examples/control_backdrop");
        let child = Command::new(&exe).arg(pattern).spawn().unwrap_or_else(|e| {
            panic!(
                "spawn {} — run `cargo build --example control_backdrop` first: {e}",
                exe.display()
            )
        });
        std::thread::sleep(STARTUP_GRACE);
        Self(child)
    }
}

const BACKDROP_DARK_LUMA: f64 = 20.0;
const BACKDROP_LIGHT_LUMA: f64 = 200.0;
const BACKDROP_CHECKER_LUMA: f64 = (BACKDROP_DARK_LUMA + BACKDROP_LIGHT_LUMA) / 2.0;
const BACKDROP_TOLERANCE: f64 = 4.0;

fn high_frequency_energy(img: &image::RgbaImage, x0: u32, y0: u32, x1: u32, y1: u32) -> f64 {
    let luma = |x: u32, y: u32| {
        let [r, g, b, _] = img.get_pixel(x, y).0;
        0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b)
    };
    let mut total = 0.0;
    let mut n = 0u32;
    for y in y0 + 1..y1 - 1 {
        for x in x0 + 1..x1 - 1 {
            total += (4.0 * luma(x, y)
                - luma(x - 1, y)
                - luma(x + 1, y)
                - luma(x, y - 1)
                - luma(x, y + 1))
            .abs();
            n += 1;
        }
    }
    total / f64::from(n.max(1))
}

fn mean_luma(img: &image::RgbaImage, x0: u32, y0: u32, x1: u32, y1: u32) -> f64 {
    let mut total = 0.0;
    let mut n = 0u32;
    for y in y0..y1 {
        for x in x0..x1 {
            let [r, g, b, _] = img.get_pixel(x, y).0;
            total += 0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b);
            n += 1;
        }
    }
    total / f64::from(n.max(1))
}

fn measure_over(pattern: &str) -> (f64, f64, f64, f64) {
    let _backdrop = Backdrop::up(pattern);
    let app = SandboxedApp::launch();
    let (window, own) = capture_card(&app);

    let own_scale = capture_scale(&own, &window);
    let (cx0, cy0, cx1, cy1) = card_bbox(&own);
    let card = Rect {
        x: window.rect.x + f64::from(cx0) / own_scale,
        y: window.rect.y + f64::from(cy0) / own_scale,
        w: f64::from(cx1 - cx0) / own_scale,
        h: f64::from(cy1 - cy0) / own_scale,
    };

    const MARGIN: f64 = 40.0;
    let region = Rect {
        x: card.x - MARGIN,
        y: card.y - MARGIN,
        w: card.w + 2.0 * MARGIN,
        h: card.h + 2.0 * MARGIN,
    };
    let shot = app.home.path().join("over.png");
    capture_region(region, &shot);
    let img = image::open(&shot)
        .expect("region capture is not a decodable image")
        .to_rgba8();
    let scale = f64::from(img.width()) / region.w;
    let px = |v: f64| (v * scale).round() as u32;

    let inside = (
        px(MARGIN + card.w * 0.45),
        px(MARGIN + card.h * 0.42),
        px(MARGIN + card.w * 0.95),
        px(MARGIN + card.h * 0.58),
    );
    let outside = (
        px(MARGIN),
        px(4.0),
        px(MARGIN + card.w * 0.5),
        px(4.0) + px(MARGIN * 0.6),
    );

    (
        mean_luma(&img, inside.0, inside.1, inside.2, inside.3),
        high_frequency_energy(&img, inside.0, inside.1, inside.2, inside.3),
        high_frequency_energy(&img, outside.0, outside.1, outside.2, outside.3),
        mean_luma(&img, outside.0, outside.1, outside.2, outside.3),
    )
}

#[test]
#[ignore = "needs a GUI session + Screen Recording grant; run via --ignored"]
fn card_is_frosted_glass_not_a_tinted_pane() {
    let (bright_mean, bright_hf, control_hf, control_mean) = measure_over("checker");
    eprintln!(
        "[frost] over checker: card mean={bright_mean:.1} hf={bright_hf:.2} | \
         backdrop mean={control_mean:.1} hf={control_hf:.1}"
    );
    assert!(
        control_hf > 20.0 && (control_mean - BACKDROP_CHECKER_LUMA).abs() < BACKDROP_TOLERANCE,
        "the checkerboard is not behind the popup (backdrop reads mean {control_mean:.1}, \
         hf {control_hf:.1}) — nothing below this measures the card, it measures the \
         developer's desktop. Build the instrument first: \
         `cargo build --example control_backdrop`."
    );

    let (dark_mean, _, _, dark_control) = measure_over("dark");
    eprintln!("[frost] over dark: card mean={dark_mean:.1} | backdrop mean={dark_control:.1}");
    assert!(
        (dark_control - BACKDROP_DARK_LUMA).abs() < BACKDROP_TOLERANCE,
        "the dark control field is not behind the popup either (reads {dark_control:.1}, \
         want {BACKDROP_DARK_LUMA:.0})"
    );

    assert!(
        bright_hf / control_hf < 0.02,
        "the backdrop's detail survives behind the card ({:.3} of its energy, need under \
         0.02) — the surface is dimming what is behind it, not blurring it. The frosted \
         panel comes from `platform::sync_card_backdrop`; check it is inserted below the \
         GL view and not above it.",
        bright_hf / control_hf
    );

    assert!(
        bright_mean - dark_mean > 5.0,
        "the card reads the same over a bright backdrop ({bright_mean:.1}) as over a dark \
         one ({dark_mean:.1}) — it is opaque, and the blur assertion above passes on an \
         opaque card for free. Spec §2 wants what is behind the window to show through."
    );
}

#[test]
#[ignore = "needs a GUI session + Screen Recording grant; run via --ignored"]
fn card_casts_one_soft_shadow_and_no_hard_rim() {
    let _backdrop = Backdrop::up("light");
    let app = SandboxedApp::launch();
    let (window, own) = capture_card(&app);

    let own_scale = capture_scale(&own, &window);
    let (cx0, cy0, cx1, _) = card_bbox(&own);
    let card_top = window.rect.y + f64::from(cy0) / own_scale;
    let card_mid_x = window.rect.x + f64::from(cx0 + cx1) / (2.0 * own_scale);

    const REACH: f64 = 40.0;
    let strip = Rect {
        x: card_mid_x - 8.0,
        y: card_top - REACH,
        w: 16.0,
        h: REACH,
    };
    let shot = app.home.path().join("above.png");
    capture_region(strip, &shot);
    let img = image::open(&shot)
        .expect("strip capture is not a decodable image")
        .to_rgba8();
    let (w, h) = img.dimensions();

    let profile: Vec<f64> = (0..h).map(|y| mean_luma(&img, 0, y, w, y + 1)).collect();

    let far = profile[0];
    let far_rows = &profile[..profile.len() / 4];
    let off = far_rows
        .iter()
        .map(|l| (l - BACKDROP_LIGHT_LUMA).abs())
        .fold(0.0_f64, f64::max);
    assert!(
        off < BACKDROP_TOLERANCE,
        "the far end of the strip strays {off:.0} levels from the flat light backdrop \
         ({BACKDROP_LIGHT_LUMA:.0}) — either the pattern is not behind the popup, \
         something else is on screen over it, or the shadow reaches further than \
         {}pt, and nothing below this measures the shadow's shape",
        REACH * 0.75
    );

    let (mut worst, mut worst_y) = (0.0_f64, 0usize);
    for (i, pair) in profile
        .iter()
        .rev()
        .collect::<Vec<_>>()
        .windows(2)
        .enumerate()
    {
        let fall = pair[0] - pair[1];
        if fall > worst {
            worst = fall;
            worst_y = i + 1;
        }
    }
    eprintln!(
        "[shadow] far={far:.0} near={:.0} largest reversal {worst:.1} at {worst_y}px above the card",
        profile[profile.len() - 1],
    );
    eprintln!(
        "[shadow] profile (px above card -> luma): {}",
        (0..profile.len())
            .rev()
            .step_by(2)
            .map(|i| format!("{}:{:.0}", profile.len() - i, profile[i]))
            .collect::<Vec<_>>()
            .join(" ")
    );

    assert!(
        worst < 6.0,
        "the backdrop gets darker again {worst_y}px away from the card, by {worst:.1} levels — \
         a shadow only ever lightens with distance, so this is a second one, cast by a \
         silhouette that is not the card. On macOS that means the renderer and the \
         compositor are both drawing: `CARD_SHADOW` in src/app.rs must be NONE wherever \
         `sync_card_backdrop` calls setHasShadow(true)."
    );
}

const JUDGE_PROMPT: &str = "\
The image is a screenshot of a small 'spotlight'-style popup card for a \
journaling app, floating over a checkerboard test pattern. The card is a dark \
rounded panel with three stacked rows: a text field labelled 'Что я делаю?', a \
text field labelled 'Хуйня?', and a number field (e.g. '10'). The first row is \
focused and so has a lighter background.\n\n\
The checkerboard is a deliberate test pattern, not part of the app. The card \
is a frosted-glass surface, so the checkerboard is expected to be blurred away \
to a smooth tone behind it — that is correct, never fail for it.\n\n\
Judge the CARD as a piece of visual craft, as a designer reviewing a build \
would. Fail it for anything that looks unfinished or broken: misaligned or \
doubled outlines, corners that do not match, edges that stop short or overrun, \
uneven padding, a row that differs from its neighbours without reason, \
clipped or overlapping text, or a boundary you cannot find. Do not invent \
faults — a plain, clean, unremarkable card passes.\n\n\
Respond with ONLY a JSON object, no prose: \
{\"pass\": true|false, \"reason\": \"<one concise sentence>\"}.";

#[test]
#[ignore = "needs a GUI session, Screen Recording, an OPENAI_API_KEY + network; run via --ignored"]
fn card_passes_the_vision_judge() {
    let _backdrop = Backdrop::up("checker");
    let app = SandboxedApp::launch();
    let (window, _) = capture_card(&app);

    let shot = app.home.path().join("judged.png");
    capture_region(window.rect, &shot);
    if let Ok(path) = std::env::var("NUDGE_DUMP_PNG") {
        std::fs::copy(&shot, &path).expect("copy judged image");
        eprintln!("[llm-judge] wrote judged image to {path}");
    }
    let png = std::fs::read(&shot).expect("read judged image");

    let Some((pass, reason)) = common::vision_verdict(&png, JUDGE_PROMPT) else {
        return;
    };
    assert!(pass, "LLM vision judge FAILED: {reason}");
}

// winit on macOS maps key code 0x24 to NamedKey::Enter and, for a named key,
// ignores the event's characters: Return comes from the key code alone.
// proof: https://docs.rs/crate/winit/0.30.13/source/src/platform_impl/macos/event.rs
const KEYCODE_RETURN: CGKeyCode = 36;

fn event_source() -> core_graphics::event_source::CGEventSource {
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
    CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .expect("CGEventSourceCreate failed — is there a login session?")
}

fn type_text(text: &str) {
    use core_graphics::event::{CGEvent, CGEventTapLocation};

    for ch in text.chars() {
        let mut buf = [0u16; 2];
        let utf16 = ch.encode_utf16(&mut buf);
        for down in [true, false] {
            let event = CGEvent::new_keyboard_event(event_source(), 0, down)
                .expect("CGEventCreateKeyboardEvent failed");
            event.set_string_from_utf16_unchecked(utf16);
            event.post(CGEventTapLocation::HID);
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}

fn press_return() {
    use core_graphics::event::{CGEvent, CGEventTapLocation};

    for down in [true, false] {
        let event = CGEvent::new_keyboard_event(event_source(), KEYCODE_RETURN, down)
            .expect("CGEventCreateKeyboardEvent failed");
        event.post(CGEventTapLocation::HID);
        std::thread::sleep(Duration::from_millis(60));
    }
}

#[test]
#[ignore = "needs a GUI session + PostEvent grant; run in the guest via scripts/native-tests-macos.sh"]
fn popup_reappears_at_its_place_after_a_reveal_desktop_sweep() {
    let display = main_display_frame();
    let work = work_area();
    let probe = foreign_probe_window(&display).expect(
        "no on-screen window of another app to watch — the reveal-desktop sweep \
         cannot be observed without one (the guest normally has Terminal/Finder up)",
    );
    eprintln!("probe window {} at {:?}", probe.0, probe.1);

    let app = SandboxedApp::launch();
    assert!(
        popup_visible(app.pid()),
        "the popup never appeared — harness problem, not a placement problem"
    );
    dismiss_with_escape();
    assert!(
        wait_until(Duration::from_secs(1), || !popup_visible(app.pid())),
        "Esc did not dismiss the popup — synthetic input is not reaching the app"
    );

    let wallpaper = (display.x + 4.0, display.y + display.h / 2.0);
    let covering = windows_at(wallpaper);
    assert!(
        covering.is_empty(),
        "the wallpaper point {wallpaper:?} is covered by {covering:?} — the click \
         would land on a window, not on the desktop"
    );
    let _restore = RevealDesktopGuard {
        probe_id: probe.0,
        home: probe.1,
        wallpaper,
    };
    click_at(wallpaper);
    assert!(
        wait_until(Duration::from_secs(2), || {
            window_rect_by_id(probe.0).is_some_and(|r| r != probe.1)
        }),
        "guest is not in reveal-desktop mode: a wallpaper click left {:?} where it was. \
         Without the sweep this test proves nothing. A nudge killed while its status-item \
         menu was still fading out — a status-item test that failed just before this one — \
         leaves the guest's wallpaper click dead, with no nudge running, until a status-item \
         menu is used to completion again.",
        probe.1
    );

    press_chord(
        VK_SPACE,
        CGEventFlags::CGEventFlagControl | CGEventFlags::CGEventFlagShift,
    );
    let shown = wait_until(Duration::from_secs(2), || popup_visible(app.pid()));
    std::thread::sleep(Duration::from_millis(500));
    let reshown = window_rect(app.pid()).expect("nudge window vanished").rect;
    assert!(
        window_rect_by_id(probe.0).is_some_and(|r| r != probe.1),
        "reveal mode ended before the popup came back, so the re-show below does \
         not exercise it"
    );
    eprintln!(
        "after ⌃⇧Space: visible={shown} rect={reshown:?}; probe window at {:?} (home {:?})",
        window_rect_by_id(probe.0),
        probe.1
    );
    assert!(shown, "⌃⇧Space did not bring the popup back at all");
    assert!(
        reshown.intersects(&display),
        "the popup came back off-screen at {reshown:?}, where the reveal-desktop sweep \
         put it — visible to the window server, invisible to the user"
    );
    assert!(
        reshown.same_as(&work),
        "the popup came back on screen at {reshown:?}, not as the work area {work:?} \
         (spec §1)"
    );

    let mut drift = Vec::new();
    let settle_until = std::time::Instant::now() + Duration::from_secs(1);
    while std::time::Instant::now() < settle_until {
        if let Some(w) = window_rect(app.pid())
            && displaced(&w.rect, &reshown)
        {
            drift.push(w.rect);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        drift.is_empty(),
        "the re-shown popup did not stay at its place: {drift:?}"
    );
}

#[test]
#[ignore = "needs a GUI session + PostEvent grant; run in the guest via scripts/native-tests-macos.sh"]
fn popup_takes_the_display_of_the_frontmost_window() {
    let display = main_display_frame();
    let (probe_pid, probe) = front_probe_window().expect(
        "no on-screen window of another app to put in front — the rule cannot be \
         exercised without one (the guest normally has Terminal/Finder up)",
    );
    let probe_centre = (probe.x + probe.w / 2.0, probe.y + probe.h / 2.0);
    assert!(
        probe_centre.0 >= display.x
            && probe_centre.0 < display.x + display.w
            && probe_centre.1 >= display.y
            && probe_centre.1 < display.y + display.h,
        "the probe window {probe:?} is centred off the main display {display:?} — the \
         guest is expected to have one display, and the expectation below is that \
         display's work area"
    );
    let work = work_area();

    let app = SandboxedApp::launch();
    assert!(
        popup_visible(app.pid()),
        "the popup never appeared — harness problem, not a placement problem"
    );
    dismiss_with_escape();
    assert!(
        wait_until(Duration::from_secs(1), || !popup_visible(app.pid())),
        "Esc did not dismiss the popup — synthetic input is not reaching the app"
    );

    bring_frontmost(probe_pid);
    assert!(
        wait_until(Duration::from_secs(2), || frontmost_pid()
            == Some(probe_pid)),
        "pid {probe_pid} did not become the frontmost application (frontmost is {:?}) \
         — without that the chord below exercises the pointer fallback, not the rule",
        frontmost_pid()
    );

    press_chord(
        VK_SPACE,
        CGEventFlags::CGEventFlagControl | CGEventFlags::CGEventFlagShift,
    );
    assert!(
        wait_until(Duration::from_secs(2), || popup_visible(app.pid())),
        "⌃⇧Space did not bring the popup back"
    );
    std::thread::sleep(Duration::from_millis(500));
    let shown = window_rect(app.pid()).expect("nudge window vanished").rect;
    eprintln!("front window {probe:?} of pid {probe_pid}; popup at {shown:?}; work {work:?}");
    assert!(
        shown.same_as(&work),
        "the popup came up at {shown:?}, not as the work area {work:?} of the display \
         holding the frontmost window {probe:?} (spec §1, which monitor)"
    );
}

fn front_probe_window() -> Option<(u32, Rect)> {
    let own = std::process::id() as i64;
    list_windows()
        .into_iter()
        .find(|w| w.layer == 0 && w.on_screen && w.owner != own && w.named && w.rect.w > 100.0)
        .map(|w| (w.owner as u32, w.rect))
}

fn bring_frontmost(pid: u32) {
    let status = Command::new("osascript")
        .arg("-e")
        .arg(format!(
            r#"tell application "System Events" to set frontmost of (first process whose unix id is {pid}) to true"#
        ))
        .status()
        .expect("osascript not runnable");
    assert!(
        status.success(),
        "System Events could not activate pid {pid}"
    );
}

#[test]
#[ignore = "needs a GUI session + PostEvent grant; run in the guest via scripts/native-tests-macos.sh"]
fn closing_the_popup_hands_the_keyboard_back_to_the_app_that_had_it() {
    let (probe_pid, probe) = front_probe_window().expect(
        "no on-screen window of another app to put in front — the hand-back cannot \
         be exercised without one (the guest normally has Terminal/Finder up)",
    );

    let app = SandboxedApp::launch();
    assert!(
        popup_visible(app.pid()),
        "the popup never appeared — harness problem, not an activation problem"
    );
    dismiss_with_escape();
    assert!(
        wait_until(Duration::from_secs(1), || !popup_visible(app.pid())),
        "Esc did not dismiss the popup — synthetic input is not reaching the app"
    );

    let closes: [(&str, fn()); 2] = [("Esc", press_escape), ("Enter", press_return)];
    for (name, close) in closes {
        bring_frontmost(probe_pid);
        assert!(
            wait_until(Duration::from_secs(2), || frontmost_pid()
                == Some(probe_pid)),
            "pid {probe_pid} ({probe:?}) did not become the frontmost application \
             (frontmost is {:?}) — nothing to hand the keyboard back to",
            frontmost_pid()
        );

        press_chord(
            VK_SPACE,
            CGEventFlags::CGEventFlagControl | CGEventFlags::CGEventFlagShift,
        );
        assert!(
            wait_until(Duration::from_secs(2), || popup_visible(app.pid())),
            "⌃⇧Space did not bring the popup back before {name}"
        );
        assert!(
            wait_until(Duration::from_secs(2), || frontmost_pid()
                == Some(app.pid())),
            "the popup came up but nudge is not the frontmost application (frontmost \
             is {:?}) — it never took the keyboard, so {name} would go to the probe",
            frontmost_pid()
        );
        std::thread::sleep(Duration::from_millis(300));

        if name == "Enter" {
            type_text("handing back");
        }
        close();
        assert!(
            wait_until(Duration::from_secs(2), || !popup_visible(app.pid())),
            "{name} did not close the popup — the keyboard is not reaching it"
        );
        assert!(
            wait_until(Duration::from_secs(2), || frontmost_pid()
                == Some(probe_pid)),
            "after {name} the frontmost application is {:?}, not pid {probe_pid} that \
             had the keyboard before the hotkey — the popup kept the activation \
             (spec §1: hands it back to the application that had it)",
            frontmost_pid()
        );
    }
}

#[test]
#[ignore = "needs a GUI session + PostEvent grant; run in the guest via scripts/native-tests-macos.sh"]
fn a_click_beside_the_card_closes_the_popup_and_reaches_nothing() {
    let display = main_display_frame();
    let work = work_area();
    let probe = foreign_probe_window(&display).expect(
        "no on-screen window of another app to watch — a click reaching the desktop \
         cannot be observed without one (the guest normally has Terminal/Finder up)",
    );

    let app = SandboxedApp::launch();
    assert!(
        popup_visible(app.pid()),
        "the popup never appeared — harness problem, not a shield problem"
    );
    let shield = window_rect(app.pid()).expect("no nudge window found");

    let point = (work.x + 4.0, work.y + work.h / 2.0);
    let covering = windows_at(point);
    assert!(
        covering.iter().any(|w| w.id == shield.id),
        "the popup's window is not under {point:?} ({covering:?}) — it does not reach \
         the work area's edge, so a click there would reach the desktop"
    );
    assert!(
        covering.iter().all(|w| w.owner == i64::from(app.pid())),
        "another window sits under {point:?} as well ({covering:?}); a click getting \
         past the shield would land on it rather than on the wallpaper, and the probe \
         below could not tell"
    );
    let _restore = RevealDesktopGuard {
        probe_id: probe.0,
        home: probe.1,
        wallpaper: point,
    };
    click_at(point);

    assert!(
        wait_until(Duration::from_secs(1), || !popup_visible(app.pid())),
        "a click beside the card left the popup open"
    );
    std::thread::sleep(Duration::from_secs(1));
    assert_eq!(
        window_rect_by_id(probe.0),
        Some(probe.1),
        "the click reached the wallpaper: the desktop was revealed and window {} moved. \
         Spec §1: the window covers the work area while visible so that it does not.",
        probe.0
    );
}

struct SpriteShot {
    window: WindowInfo,
    img: image::RgbaImage,
    scale: f64,
    card: (u32, u32, u32, u32),
    sprite: Option<(u32, u32, u32, u32)>,
}

fn shoot(app: &SandboxedApp) -> SpriteShot {
    let (window, img) = capture_card(app);
    let scale = capture_scale(&img, &window);
    let card = card_bbox(&img);
    let sprite = sprite_bbox(&img);
    eprintln!(
        "window={:?} scale={scale} card={card:?} sprite={sprite:?}",
        window.rect
    );
    SpriteShot {
        window,
        img,
        scale,
        card,
        sprite,
    }
}

#[test]
#[ignore = "needs a GUI session + Screen Recording grant; run via --ignored"]
fn sprite_window_is_still_the_work_area() {
    let app = SandboxedApp::launch_with_config(&config_with_sprite(true, -41, -277, 679));
    let window = window_rect(app.pid()).expect("no nudge window found");
    let work = work_area();
    eprintln!("window={:?} work={work:?}", window.rect);
    assert!(
        popup_visible(app.pid()),
        "the popup never appeared with a sprite configured"
    );
    assert!(
        window.rect.same_as(&work),
        "with a sprite the window is {:?}, not the work area {work:?} — spec §10: the \
         window covers the work area; the card and the sprite are drawn inside it",
        window.rect
    );
}

#[test]
#[ignore = "needs a GUI session + Screen Recording grant; run via --ignored"]
fn sprite_pixels_are_drawn_over_the_card_and_transparent_outside() {
    let (fixture, (fx0, fy0, fx1, fy1)) = sprite_fixture();
    let (mid_x, mid_y) = ((fx0 + fx1) / 2, (fy0 + fy1) / 2);
    let body = fixture
        .enumerate_pixels()
        .filter(|(_, _, p)| is_green(p))
        .min_by_key(|(x, y, _)| x.abs_diff(mid_x) + y.abs_diff(mid_y))
        .map(|(x, y, _)| (x, y))
        .expect("the fixture has opaque pixels");
    let scale = display_scale();
    const INTO_CARD: (f64, f64) = (60.0, 40.0);
    let x = (INTO_CARD.0 - f64::from(body.0) / scale).round() as i32;
    let y = (INTO_CARD.1 - f64::from(body.1) / scale).round() as i32;
    let app = SandboxedApp::launch_with_config(&config_with_sprite(true, x, y, 480));
    let shot = shoot(&app);
    let (cx0, cy0, cx1, cy1) = shot.card;
    let (sx0, sy0, sx1, sy1) = shot.sprite.expect("no sprite pixels on the window at all");
    let px = |v: f64| (v * shot.scale).round() as u32;

    assert_eq!(
        (sx1 - sx0, sy1 - sy0),
        (fx1 - fx0, fy1 - fy0),
        "the sprite's opaque pixels span {}×{} capture pixels, the fixture's {}×{} — \
         spec §10: 1 image pixel = 1 device pixel, never resampled (or the sprite runs \
         off the window: it starts at ({x}, {y}) from the card)",
        sx1 - sx0 + 1,
        sy1 - sy0 + 1,
        fx1 - fx0 + 1,
        fy1 - fy0 + 1
    );
    let at = |(fx, fy): (u32, u32)| (sx0 - fx0 + fx, sy0 - fy0 + fy);

    let (bx, by) = at(body);
    assert!(
        (cx0..=cx1).contains(&bx) && (cy0..=cy1).contains(&by),
        "the probe landed at ({bx}, {by}), outside the card {:?} — the placement under \
         test is not the one meant",
        shot.card
    );
    let over = shot.img.get_pixel(bx, by);
    assert!(
        is_green(over),
        "{}×{} pt into the card, under the sprite, the pixel is {over:?} — the sprite \
         is not drawn over the card (spec §10: it is drawn in front)",
        INTO_CARD.0,
        INTO_CARD.1
    );
    let (mx, my) = at((fx0 + 20, fy0 - 3));
    assert!(
        my < cy0,
        "the sprite's top is not above the card: {my} vs {cy0}"
    );
    let margin = shot.img.get_pixel(mx, my);
    assert_eq!(
        margin.0[3], 0,
        "the sprite's transparent surround reads {margin:?} — something is painted \
         behind the sprite (spec §10: no material and no fill beyond the card)"
    );
    let bare = shot.img.get_pixel(cx1 - px(40.0), (cy0 + cy1) / 2);
    assert!(
        bare.0[3] > 128 && !is_green(bare),
        "the card beside the sprite reads {bare:?} — the sprite is covering more than \
         its own pixels"
    );
}

#[test]
#[ignore = "needs a GUI session + Screen Recording grant; run via --ignored"]
fn sprite_keeps_the_card_at_its_spec_anchor() {
    let display = main_display_frame();
    let work = work_area();
    let (sw, _) = sprite_points();
    let anchor = card_anchor_top(&display);
    let y = -(anchor - work.y) as i32 + 20;
    let app =
        SandboxedApp::launch_with_config(&config_with_sprite(true, -(sw + 10.0) as i32, y, 480));
    let shot = shoot(&app);
    let card = bbox_rect(&shot.window, shot.scale, shot.card);
    let (_, sy0, _, _) = shot.sprite.expect("no sprite pixels on the window at all");

    let sprite_top = f64::from(sy0 - sprite_opaque_top()) / shot.scale;
    assert!(
        (sprite_top - 20.0).abs() <= 1.0,
        "the sprite's top is {sprite_top} pt below the window's top, not 20 — the \
         placement under test is not the one configured"
    );
    assert!(
        (card.y - anchor).abs() <= 1.0,
        "the card's top is at y={}, its §1 anchor is y={anchor} — a sprite inside the \
         work area must not move the card (spec §10)",
        card.y
    );
}

#[test]
#[ignore = "needs a GUI session + Screen Recording grant; run via --ignored"]
fn sprite_overhanging_the_work_area_moves_the_card_down() {
    let display = main_display_frame();
    let work = work_area();
    let (sw, _) = sprite_points();
    let anchor = card_anchor_top(&display);
    const OVERHANG: f64 = 30.0;
    let y = -(anchor - work.y + OVERHANG) as i32;
    let app =
        SandboxedApp::launch_with_config(&config_with_sprite(true, -(sw + 10.0) as i32, y, 480));
    let shot = shoot(&app);
    let card = bbox_rect(&shot.window, shot.scale, shot.card);
    let (_, sy0, _, _) = shot.sprite.expect("no sprite pixels on the window at all");

    assert!(
        (f64::from(sy0 - sprite_opaque_top()) / shot.scale).abs() <= 1.0,
        "the sprite's top is {} capture pixels below the window's top; with the card \
         moved down by the overhang it starts at the work area's top (spec §10)",
        sy0 - sprite_opaque_top()
    );
    assert!(
        (card.y - (anchor + OVERHANG)).abs() <= 1.0,
        "the card's top is at y={}; its anchor is y={anchor} and the sprite overhangs \
         the work area by {OVERHANG} pt, so spec §10 puts it at y={}",
        card.y,
        anchor + OVERHANG
    );
}

#[test]
#[ignore = "needs a GUI session + Screen Recording grant; run via --ignored"]
fn sprite_off_shows_the_plain_card() {
    let app = SandboxedApp::launch_with_config(&config_with_sprite(false, -41, -277, 679));
    let shot = shoot(&app);
    assert_eq!(
        shot.sprite, None,
        "the sprite is drawn although `enabled` is false"
    );
    let card = bbox_rect(&shot.window, shot.scale, shot.card);
    assert!(
        (card.w - 480.0).abs() <= 1.0,
        "the card is {} pt wide; with the sprite off `card_width` is ignored and §2's \
         480 applies",
        card.w
    );
}

#[test]
#[ignore = "needs a GUI session + Screen Recording grant; run via --ignored"]
fn sprite_with_a_bad_png_falls_back_and_logs() {
    let dir = tempfile::tempdir().expect("temp dir");
    let rgb = dir.path().join("rgb.png");
    image::RgbImage::from_pixel(8, 8, image::Rgb([0, 255, 0]))
        .save(&rgb)
        .expect("write an RGB PNG");
    let mut app = SandboxedApp::launch_with_config_logging(
        &config_with_sprite(true, -41, -277, 679).replace("{sprite}", &rgb.display().to_string()),
    );

    let shot = shoot(&app);
    assert_eq!(
        shot.sprite, None,
        "a PNG without alpha was drawn as a sprite"
    );
    let card = bbox_rect(&shot.window, shot.scale, shot.card);
    assert!(
        (card.w - 480.0).abs() <= 1.0,
        "the card is {} pt wide; a sprite that failed the check counts as off, so \
         `card_width` must not apply",
        card.w
    );

    let log = app.stderr_after_exit();
    assert!(
        log.contains("sprite not shown") && log.contains("no alpha channel"),
        "the app did not say why the sprite is not shown; stderr:\n{log}"
    );
}

#[test]
#[ignore = "needs a GUI session + Screen Recording + PostEvent grants; run in the guest via scripts/native-tests-macos.sh"]
fn sprite_change_applies_while_hidden_and_waits_while_visible() {
    let app = SandboxedApp::launch_with_config(&config_with_sprite(false, -41, -277, 679));
    let before = shoot(&app);
    assert_eq!(
        before.sprite, None,
        "the sprite is on before it was turned on"
    );

    let (sw, _) = sprite_points();
    std::fs::write(
        app.config_path(),
        config_with_sprite(true, -(sw + 10.0) as i32, -20, 679)
            .replace("{sprite}", &app.sprite_path().display().to_string()),
    )
    .expect("edit config.json in place");

    let settle_until = std::time::Instant::now() + Duration::from_secs(3);
    while std::time::Instant::now() < settle_until {
        let now = shoot(&app);
        assert_eq!(
            now.sprite, None,
            "the sprite appeared under the user's hands — spec §10 applies a change \
             only while the popup is hidden"
        );
        assert_eq!(
            now.card, before.card,
            "the card changed under the user's hands — spec §10 applies a change only \
             while the popup is hidden"
        );
        std::thread::sleep(Duration::from_millis(300));
    }

    dismiss_with_escape();
    assert!(
        wait_until(Duration::from_secs(1), || !popup_visible(app.pid())),
        "Esc did not dismiss the popup — synthetic input is not reaching the app"
    );
    press_chord(
        VK_SPACE,
        CGEventFlags::CGEventFlagControl | CGEventFlags::CGEventFlagShift,
    );
    assert!(
        wait_until(Duration::from_secs(2), || popup_visible(app.pid())),
        "⌃⇧Space did not bring the popup back"
    );
    std::thread::sleep(Duration::from_millis(500));

    let after = shoot(&app);
    assert!(
        after.sprite.is_some(),
        "the popup came back without the sprite that was saved while it was visible"
    );
    let card = bbox_rect(&after.window, after.scale, after.card);
    assert!(
        (card.w - 679.0).abs() <= 1.0,
        "the card came back {} pt wide, not the 679 saved while the popup was visible",
        card.w
    );
}

#[test]
#[ignore = "needs a GUI session + PostEvent grant; run in the guest via scripts/native-tests-macos.sh"]
fn launchd_start_under_activation_contention_shows_nothing_until_a_key_popup_at_the_deadline() {
    const INTERVAL: Duration = Duration::from_secs(3);
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_sampler = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let job = LaunchdApp::bootstrap(0.05);

    let switcher = {
        let stop = stop.clone();
        std::thread::spawn(move || {
            let mut presses = 0u32;
            while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                quick_cmd_tab();
                presses += 1;
                std::thread::sleep(Duration::from_millis(10));
            }
            presses
        })
    };
    let fronts = {
        let stop = stop_sampler.clone();
        std::thread::spawn(move || {
            let t0 = std::time::Instant::now();
            let mut seen: Vec<(u128, Option<u32>)> = Vec::new();
            while !stop.load(std::sync::atomic::Ordering::SeqCst) {
                let f = frontmost_pid();
                if seen.last().map(|s| s.1) != Some(f) {
                    seen.push((t0.elapsed().as_millis(), f));
                }
            }
            seen
        })
    };
    let _cmd_up = ReleaseModifiers;

    let pid = job.wait_for_pid(Duration::from_secs(10));
    let at_launch = wait_until(INTERVAL - Duration::from_secs(1), || {
        popup_stays_visible(pid)
    });
    let appeared = at_launch || wait_until(Duration::from_secs(10), || popup_visible(pid));
    std::thread::sleep(Duration::from_millis(300));
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let presses = switcher.join().expect("switcher thread");
    assert!(
        !at_launch,
        "a popup came on screen while the login item started — spec §4 starts in \
         Waiting; stderr:\n{}",
        job.stderr()
    );
    assert!(
        appeared,
        "the first deadline never showed the popup; stderr:\n{}",
        job.stderr()
    );
    eprintln!("Cmd-Tab presses during launch and the first deadline: {presses}");

    std::thread::sleep(Duration::from_secs(1));
    stop_sampler.store(true, std::sync::atomic::Ordering::SeqCst);
    let fronts = fronts.join().expect("frontmost sampler");
    eprintln!("frontmost over time (ms since bootstrap, pid): {fronts:?}");
    let visible = popup_visible(pid);
    let front = frontmost_pid();
    eprintln!("1s after contention: visible={visible} frontmost={front:?} nudge={pid}");
    assert!(
        !visible || front == Some(pid),
        "the popup is on screen but its app is not frontmost (frontmost pid {front:?}, \
         nudge {pid}): it never became key, so focus loss cannot close it and Esc \
         goes to another app"
    );

    press_escape();
    assert!(
        wait_until(Duration::from_secs(1), || !popup_visible(pid)),
        "a raw Esc did not dismiss the popup — the keyboard is not reaching it"
    );
}

fn displaced(a: &Rect, b: &Rect) -> bool {
    let centre = |r: &Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let ((ax, ay), (bx, by)) = (centre(a), centre(b));
    (ax - bx).abs() > 4.0 || (ay - by).abs() > 4.0
}

fn foreign_probe_window(display: &Rect) -> Option<(u32, Rect)> {
    let own = std::process::id() as i64;
    list_windows()
        .into_iter()
        .find(|w| w.layer == 0 && w.on_screen && w.owner != own && w.named && w.rect.w > 100.0)
        .filter(|w| w.rect.intersects(display))
        .map(|w| (w.id, w.rect))
}

fn windows_at(p: (f64, f64)) -> Vec<ListedWindow> {
    list_windows()
        .into_iter()
        .filter(|w| w.on_screen && (0..20).contains(&w.layer))
        .filter(|w| {
            p.0 >= w.rect.x
                && p.0 < w.rect.x + w.rect.w
                && p.1 >= w.rect.y
                && p.1 < w.rect.y + w.rect.h
        })
        .collect()
}

fn window_rect_by_id(id: u32) -> Option<Rect> {
    list_windows()
        .into_iter()
        .find(|w| w.id == id)
        .map(|w| w.rect)
}

#[derive(Debug)]
struct ListedWindow {
    id: u32,
    owner: i64,
    layer: i64,
    rect: Rect,
    on_screen: bool,
    named: bool,
}

fn list_windows() -> Vec<ListedWindow> {
    let Some(windows) = CGDisplay::window_list_info(kCGWindowListOptionAll, None) else {
        return Vec::new();
    };
    let key = |k| unsafe { CFString::wrap_under_get_rule(k) };
    let num = |d: &CFDictionary<CFString, CFType>, k| {
        d.find(key(k))
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|n| n.to_i64())
    };
    let mut out = Vec::new();
    for i in 0..windows.len() {
        let Some(entry) = windows.get(i) else {
            continue;
        };
        let d: CFDictionary<CFString, CFType> =
            unsafe { TCFType::wrap_under_get_rule(*entry as CFDictionaryRef) };
        let Some(b) = d
            .find(key(unsafe { kCGWindowBounds }))
            .and_then(|v| v.downcast::<CFDictionary>())
            .and_then(|b| CGWindowRect::from_dict_representation(&b))
        else {
            continue;
        };
        out.push(ListedWindow {
            id: num(&d, unsafe { kCGWindowNumber }).unwrap_or(0) as u32,
            owner: num(&d, unsafe { kCGWindowOwnerPID }).unwrap_or(-1),
            layer: num(&d, unsafe { kCGWindowLayer }).unwrap_or(0),
            rect: Rect {
                x: b.origin.x,
                y: b.origin.y,
                w: b.size.width,
                h: b.size.height,
            },
            on_screen: d
                .find(key(unsafe { kCGWindowIsOnscreen }))
                .and_then(|v| v.downcast::<CFBoolean>())
                .map(bool::from)
                .unwrap_or(false),
            named: d
                .find(key(unsafe { kCGWindowName }))
                .and_then(|v| v.downcast::<CFString>())
                .is_some_and(|s| !s.to_string().is_empty()),
        });
    }
    out
}

struct RevealDesktopGuard {
    probe_id: u32,
    home: Rect,
    wallpaper: (f64, f64),
}

impl Drop for RevealDesktopGuard {
    fn drop(&mut self) {
        let at_home = || window_rect_by_id(self.probe_id).is_none_or(|r| r == self.home);
        if wait_until(Duration::from_millis(500), at_home) {
            return;
        }
        click_at(self.wallpaper);
        if !wait_until(Duration::from_secs(2), at_home) {
            eprintln!(
                "WARNING: the guest desktop is still in reveal mode — window {} did not return to {:?}",
                self.probe_id, self.home
            );
        }
    }
}

struct LaunchdApp {
    label: String,
    domain: String,
    home: tempfile::TempDir,
}

impl LaunchdApp {
    fn bootstrap(interval_minutes: f64) -> Self {
        let home = tempfile::tempdir().expect("temp HOME");
        std::fs::create_dir_all(home.path().join("Documents")).expect("Documents");
        let config = home.path().join(CONFIG_SUBDIR).join("config.json");
        std::fs::create_dir_all(home.path().join(CONFIG_SUBDIR)).expect("config dir");
        std::fs::write(
            &config,
            format!(
                r#"{{"hotkey": "Ctrl+Shift+Space", "default_interval_minutes": {interval_minutes}, "autostart": false}}"#
            ),
        )
        .expect("seed config.json");
        let label = "com.wizard1209.nudge.test".to_string();
        let domain = format!("gui/{}", unsafe { getuid() });
        let plist = home.path().join(format!("{label}.plist"));
        std::fs::write(
            &plist,
            format!(
                r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>Label</key><string>{label}</string>
  <key>ProgramArguments</key><array>
    <string>{bin}</string><string>--config</string><string>{config}</string>
  </array>
  <key>EnvironmentVariables</key><dict><key>HOME</key><string>{home}</string></dict>
  <key>StandardErrorPath</key><string>{home}/stderr.log</string>
  <key>RunAtLoad</key><true/>
</dict></plist>
"#,
                bin = binary_path().display(),
                config = config.display(),
                home = home.path().display(),
            ),
        )
        .expect("write plist");
        let _ = Command::new("launchctl")
            .args(["bootout", &format!("{domain}/{label}")])
            .stderr(std::process::Stdio::null())
            .status();
        let app = Self {
            label,
            domain,
            home,
        };
        let status = Command::new("launchctl")
            .arg("bootstrap")
            .arg(&app.domain)
            .arg(&plist)
            .status()
            .expect("launchctl not runnable");
        assert!(
            status.success(),
            "launchctl bootstrap {} failed",
            plist.display()
        );
        app
    }

    fn wait_for_pid(&self, timeout: Duration) -> u32 {
        let deadline = std::time::Instant::now() + timeout;
        loop {
            let out = Command::new("launchctl")
                .args(["print", &format!("{}/{}", self.domain, self.label)])
                .output()
                .expect("launchctl not runnable");
            let text = String::from_utf8_lossy(&out.stdout);
            if let Some(pid) = text
                .lines()
                .find_map(|l| l.trim().strip_prefix("pid = "))
                .and_then(|p| p.trim().parse().ok())
            {
                return pid;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "launchd never reported a pid for {}; stderr:\n{}",
                self.label,
                self.stderr()
            );
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn stderr(&self) -> String {
        std::fs::read_to_string(self.home.path().join("stderr.log")).unwrap_or_default()
    }
}

impl Drop for LaunchdApp {
    fn drop(&mut self) {
        let _ = Command::new("launchctl")
            .args(["bootout", &format!("{}/{}", self.domain, self.label)])
            .status();
    }
}

unsafe extern "C" {
    fn getuid() -> u32;
}

fn wait_until(timeout: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if done() {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn frontmost_pid() -> Option<u32> {
    let asn = Command::new("lsappinfo").arg("front").output().ok()?;
    let asn = String::from_utf8_lossy(&asn.stdout).trim().to_string();
    let info = Command::new("lsappinfo")
        .args(["info", "-only", "pid", &asn])
        .output()
        .ok()?;
    String::from_utf8_lossy(&info.stdout)
        .trim()
        .strip_prefix("\"pid\"=")?
        .parse()
        .ok()
}

fn click_at((x, y): (f64, f64)) {
    use core_graphics::event::{CGEvent, CGEventTapLocation, CGEventType, CGMouseButton};
    use core_graphics::geometry::CGPoint;

    let p = CGPoint::new(x, y);
    for (kind, pause) in [
        (CGEventType::MouseMoved, 100),
        (CGEventType::LeftMouseDown, 60),
        (CGEventType::LeftMouseUp, 60),
    ] {
        CGEvent::new_mouse_event(event_source(), kind, p, CGMouseButton::Left)
            .expect("CGEventCreateMouseEvent failed")
            .post(CGEventTapLocation::HID);
        std::thread::sleep(Duration::from_millis(pause));
    }
}

fn right_click_at((x, y): (f64, f64)) {
    use core_graphics::event::{CGEvent, CGEventTapLocation, CGEventType, CGMouseButton};
    use core_graphics::geometry::CGPoint;

    let p = CGPoint::new(x, y);
    for (kind, pause) in [
        (CGEventType::MouseMoved, 100),
        (CGEventType::RightMouseDown, 60),
        (CGEventType::RightMouseUp, 60),
    ] {
        CGEvent::new_mouse_event(event_source(), kind, p, CGMouseButton::Right)
            .expect("CGEventCreateMouseEvent failed")
            .post(CGEventTapLocation::HID);
        std::thread::sleep(Duration::from_millis(pause));
    }
}

const VK_SPACE: CGKeyCode = 0x31;
const VK_TAB: CGKeyCode = 0x30;
const VK_ESCAPE: CGKeyCode = 0x35;
const VK_COMMAND: CGKeyCode = 0x37;
const VK_SHIFT: CGKeyCode = 0x38;
const VK_OPTION: CGKeyCode = 0x3A;
const VK_CONTROL: CGKeyCode = 0x3B;
const VK_RETURN: CGKeyCode = 0x24;
const VK_J: CGKeyCode = 0x26;

// Quartz needs every keystroke of a chord posted, modifier keys included:
// the modifier goes down, the key goes down, then both are released.
// proof: https://developer.apple.com/documentation/coregraphics/cgevent/init(keyboardeventsource:virtualkey:keydown:)
fn press_chord(code: CGKeyCode, modifiers: CGEventFlags) {
    use core_graphics::event::{CGEvent, CGEventTapLocation};

    let caps: Vec<(CGEventFlags, CGKeyCode)> = [
        (CGEventFlags::CGEventFlagCommand, VK_COMMAND),
        (CGEventFlags::CGEventFlagShift, VK_SHIFT),
        (CGEventFlags::CGEventFlagAlternate, VK_OPTION),
        (CGEventFlags::CGEventFlagControl, VK_CONTROL),
    ]
    .into_iter()
    .filter(|(flag, _)| modifiers.contains(*flag))
    .collect();

    let mut held = CGEventFlags::CGEventFlagNull;
    for (flag, cap) in &caps {
        held |= *flag;
        post_flags_changed(*cap, held, 60);
    }
    for down in [true, false] {
        let event = CGEvent::new_keyboard_event(event_source(), code, down)
            .expect("CGEventCreateKeyboardEvent failed");
        event.set_flags(modifiers);
        event.post(CGEventTapLocation::HID);
        std::thread::sleep(Duration::from_millis(60));
    }
    for (flag, cap) in caps.iter().rev() {
        held.remove(*flag);
        post_flags_changed(*cap, held, 60);
    }
}

fn post_flags_changed(cap: CGKeyCode, held: CGEventFlags, pause_ms: u64) {
    use core_graphics::event::{CGEvent, CGEventTapLocation, CGEventType};

    let event = CGEvent::new_keyboard_event(event_source(), cap, true)
        .expect("CGEventCreateKeyboardEvent failed");
    event.set_flags(held);
    event.set_type(CGEventType::FlagsChanged);
    event.post(CGEventTapLocation::HID);
    std::thread::sleep(Duration::from_millis(pause_ms));
}

fn press_escape() {
    press_chord(VK_ESCAPE, CGEventFlags::CGEventFlagNull);
}

fn quick_cmd_tab() {
    use core_graphics::event::{CGEvent, CGEventTapLocation};

    post_flags_changed(VK_COMMAND, CGEventFlags::CGEventFlagCommand, 5);
    for down in [true, false] {
        let event = CGEvent::new_keyboard_event(event_source(), VK_TAB, down)
            .expect("CGEventCreateKeyboardEvent failed");
        event.set_flags(CGEventFlags::CGEventFlagCommand);
        event.post(CGEventTapLocation::HID);
        std::thread::sleep(Duration::from_millis(5));
    }
    post_flags_changed(VK_COMMAND, CGEventFlags::CGEventFlagNull, 5);
}

struct ReleaseModifiers;

impl Drop for ReleaseModifiers {
    fn drop(&mut self) {
        post_flags_changed(VK_COMMAND, CGEventFlags::CGEventFlagNull, 20);
    }
}

#[test]
#[ignore = "needs a GUI session + Screen Recording + PostEvent grants; run in the guest via scripts/native-tests-macos.sh"]
fn sprite_turned_on_while_hidden_is_drawn_at_the_next_reveal() {
    let app = SandboxedApp::launch_with_config(&config_with_sprite(false, -41, -277, 679));
    assert_eq!(
        shoot(&app).sprite,
        None,
        "the sprite is on before it was turned on"
    );

    dismiss_with_escape();
    assert!(
        wait_until(Duration::from_secs(1), || !popup_visible(app.pid())),
        "Esc did not dismiss the popup — synthetic input is not reaching the app"
    );

    let (sw, _) = sprite_points();
    std::fs::write(
        app.config_path(),
        config_with_sprite(true, -(sw + 10.0) as i32, -20, 679)
            .replace("{sprite}", &app.sprite_path().display().to_string()),
    )
    .expect("edit config.json in place");
    std::thread::sleep(Duration::from_millis(1500));

    press_chord(
        VK_SPACE,
        CGEventFlags::CGEventFlagControl | CGEventFlags::CGEventFlagShift,
    );
    assert!(
        wait_until(Duration::from_secs(2), || popup_visible(app.pid())),
        "⌃⇧Space did not bring the popup back"
    );
    std::thread::sleep(Duration::from_millis(500));

    let after = shoot(&app);
    assert!(
        after.sprite.is_some(),
        "the popup came back without the sprite that was turned on while it was hidden"
    );
}

const SETTINGS_CONTENT: (f64, f64) = (560.0, 625.0);

const TITLE_BAR_HEIGHT: f64 = 32.0;

const FILL_SATURATION: i32 = 60;

fn settings_window(pid: u32) -> Option<WindowInfo> {
    list_windows()
        .into_iter()
        .filter(|w| w.owner == i64::from(pid) && w.named)
        .max_by(|a, b| (a.rect.w * a.rect.h).total_cmp(&(b.rect.w * b.rect.h)))
        .map(|w| WindowInfo {
            id: w.id,
            rect: w.rect,
            on_screen: w.on_screen,
        })
}

fn bring_to_front(pid: u32) {
    let status = Command::new("osascript")
        .arg("-e")
        .arg(format!(
            r#"tell application "System Events" to set frontmost of (first process whose unix id is {pid}) to true"#
        ))
        .status()
        .expect("osascript not runnable");
    assert!(
        status.success(),
        "osascript failed to focus the settings process — is a GUI session active?"
    );
}

fn drag_right((x, y): (f64, f64), dx: f64) {
    use core_graphics::event::{CGEvent, CGEventTapLocation, CGEventType, CGMouseButton};
    use core_graphics::geometry::CGPoint;

    let post = |kind: CGEventType, p: CGPoint, pause: u64| {
        CGEvent::new_mouse_event(event_source(), kind, p, CGMouseButton::Left)
            .expect("CGEventCreateMouseEvent failed")
            .post(CGEventTapLocation::HID);
        std::thread::sleep(Duration::from_millis(pause));
    };
    post(CGEventType::MouseMoved, CGPoint::new(x, y), 150);
    post(CGEventType::LeftMouseDown, CGPoint::new(x, y), 100);
    for step in 1..=8 {
        let p = CGPoint::new(x + dx * f64::from(step) / 8.0, y);
        post(CGEventType::LeftMouseDragged, p, 40);
    }
    post(CGEventType::LeftMouseUp, CGPoint::new(x + dx, y), 150);
}

fn filled_block(img: &image::RgbaImage, scale: f64, below: f64) -> Option<Rect> {
    let saturated = |p: &image::Rgba<u8>| {
        let (r, g, b) = (i32::from(p[0]), i32::from(p[1]), i32::from(p[2]));
        r.max(g).max(b) - r.min(g).min(b) >= FILL_SATURATION
    };
    let min_run = (50.0 * scale) as u32;
    let min_rows = (8.0 * scale) as usize;
    let first_row = (below * scale) as u32;

    let runs: Vec<Option<(u32, u32)>> = (0..img.height())
        .map(|y| {
            if y < first_row {
                return None;
            }
            let mut best: Option<(u32, u32)> = None;
            let mut start: Option<u32> = None;
            for x in 0..=img.width() {
                let on = x < img.width() && saturated(img.get_pixel(x, y));
                match (on, start) {
                    (true, None) => start = Some(x),
                    (false, Some(s)) => {
                        if x - s >= min_run && best.is_none_or(|(bs, be)| x - s > be - bs) {
                            best = Some((s, x));
                        }
                        start = None;
                    }
                    _ => {}
                }
            }
            best
        })
        .collect();

    let mut block: Option<(usize, usize)> = None;
    let mut y = 0;
    while y < runs.len() {
        if runs[y].is_none() {
            y += 1;
            continue;
        }
        let top = y;
        while y < runs.len() && runs[y].is_some() {
            y += 1;
        }
        if y - top >= min_rows && block.is_none_or(|(t, b)| y - top > b - t) {
            block = Some((top, y));
        }
    }
    let (top, bottom) = block?;
    let x0 = (top..bottom).filter_map(|y| runs[y]).map(|r| r.0).min()?;
    let x1 = (top..bottom).filter_map(|y| runs[y]).map(|r| r.1).max()?;
    Some(Rect {
        x: f64::from(x0) / scale,
        y: top as f64 / scale,
        w: f64::from(x1 - x0) / scale,
        h: (bottom - top) as f64 / scale,
    })
}

#[test]
#[ignore = "needs a GUI session + Accessibility and Screen Recording grants; run via --ignored"]
fn settings_window_is_fixed_size_and_not_resizable() {
    let app = SandboxedApp::launch_settings();
    let window = settings_window(app.pid()).expect("no settings window found");
    eprintln!("settings window={:?}", window.rect);
    assert!(window.on_screen, "the settings window is not on screen");

    let content = (window.rect.w, window.rect.h - TITLE_BAR_HEIGHT);
    assert!(
        (content.0 - SETTINGS_CONTENT.0).abs() <= 1.0
            && (content.1 - SETTINGS_CONTENT.1).abs() <= 1.0,
        "the content area is {}×{} pt; §9 sizes the form to {}×{}",
        content.0,
        content.1,
        SETTINGS_CONTENT.0,
        SETTINGS_CONTENT.1
    );

    bring_to_front(app.pid());
    drag_right(
        (
            window.rect.x + window.rect.w - 1.0,
            window.rect.y + window.rect.h / 2.0,
        ),
        80.0,
    );
    std::thread::sleep(Duration::from_millis(300));
    let dragged = settings_window(app.pid()).expect("the settings window vanished during the drag");
    assert!(
        dragged.rect.same_as(&window.rect),
        "a drag on the right edge took the window from {:?} to {:?}: it is resizable",
        window.rect,
        dragged.rect
    );

    bring_to_front(app.pid());
    press_chord(VK_TAB, CGEventFlags::CGEventFlagNull);
    press_chord(VK_J, CGEventFlags::CGEventFlagNull);
    press_chord(VK_RETURN, CGEventFlags::CGEventFlagNull);
    std::thread::sleep(Duration::from_millis(500));
    let spoken =
        settings_window(app.pid()).expect("the settings window vanished after the refused save");
    assert!(
        spoken.rect.same_as(&dragged.rect),
        "the banner took the window from {:?} to {:?}: §9 says it never changes size",
        dragged.rect,
        spoken.rect
    );

    let img = capture_window(spoken.id, &app.home.path().join("settings.png"));
    let scale = capture_scale(&img, &spoken);
    let save = filled_block(&img, scale, TITLE_BAR_HEIGHT)
        .expect("no filled button in the window's pixels — §9 makes Save the one filled button");
    eprintln!("filled block={save:?} (window points)");
    let (w, h) = (spoken.rect.w, spoken.rect.h);
    assert!(
        save.x >= 0.0 && save.y >= 0.0 && save.x + save.w <= w && save.y + save.h <= h,
        "the filled button {save:?} runs past the {w}×{h} window"
    );
    assert!(
        save.x + save.w / 2.0 > w / 2.0 && save.y + save.h / 2.0 > h / 2.0,
        "the filled button {save:?} is not at the bottom right of the {w}×{h} window"
    );
}
