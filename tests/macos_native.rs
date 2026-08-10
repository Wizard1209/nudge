//! Native macOS harness for the real `nudge` binary.
//!
//! The macOS counterpart of `tests/native_render.rs`, and deliberately the same
//! shape: spawn the shipped binary against a throwaway HOME, locate its window,
//! capture what is actually on screen, and assert on what a user would see.
//! Only the platform calls differ — `screencapture` here where that file uses
//! GDI `BitBlt`.
//!
//! # These tests are the port's acceptance criteria
//!
//! All are green. Each names one capability the port had to restore, and none
//! can be satisfied by making the code merely compile: the harness control,
//! window placement, the interval timer, the menu-bar status item, and
//! journal-writing driven by real keystrokes.
//!
//! The appearance group below them answers a different question — not "does
//! the capability work" but "does the card look like the design says". It
//! reads the window's own pixels rather than a screen region, and where it
//! needs a backdrop it supplies one, because a translucent surface's defects
//! are all differential against what is behind it and the developer's desktop
//! is not a control.
//!
//! What they do NOT cover, because it needs synthetic *mouse* input against
//! a window the process does not own: clicking the status item to summon the
//! popup, and its right-click Show / Settings / Quit menu. The item is hosted
//! by Control Center, so `AXPress` on it does nothing — `tray-icon` overrides
//! `mouseDown:`/`mouseUp:` on a custom `NSView` and only real `CGEvent` mouse
//! events reach it.
//!
//! One environmental precondition they do not check for themselves: no other
//! `nudge` may be running. Each test launches its own, and a leftover instance
//! makes the launch popup fail to appear at all.
//!
//! # Running
//!
//! ```sh
//! cargo test --test macos_native -- --ignored --nocapture --test-threads=1
//! ```
//!
//! `--test-threads=1` is load-bearing, not tidiness. Every test launches its
//! own `nudge`, and the synthetic Esc is addressed by process *name*: with two
//! instances alive, `System Events ... process "nudge"` resolves ambiguously
//! and the keystroke can land on the wrong one. Under cargo's default
//! parallelism the suite goes flaky in the worst possible way — the failure
//! looks like a port regression rather than a harness collision.
//!
//! Requirements, all of which fail loudly rather than silently degrading:
//!   * a logged-in GUI session (no ssh-without-console, no headless CI runner);
//!   * **Screen Recording** granted to the terminal that runs `cargo` (window
//!     geometry itself comes from `CGWindowListCopyWindowInfo`, which needs no
//!     grant — see `window_rect` — but `screencapture` still does).
//!
//! That grant attaches to the *responsible* app — the terminal — not to
//! `cargo` and not to `nudge`. A fresh machine has none, which is the single
//! largest difference from the Windows harness, where `BitBlt` needs no
//! permission. Start from `harness_can_sandbox_and_capture`: it fails with a
//! pointed message when the grant is missing, so a permissions problem never
//! gets read as a port regression.

#![cfg(target_os = "macos")]

mod common;

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::Duration;

use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use core_graphics::display::{CGDisplay, kCGWindowListOptionAll};
use core_graphics::event::CGKeyCode;
use core_graphics::geometry::CGRect as CGWindowRect;
use core_graphics::window::{
    kCGWindowBounds, kCGWindowIsOnscreen, kCGWindowLayer, kCGWindowNumber, kCGWindowOwnerPID,
};

/// Wall-clock allowance for eframe to create and present its first frame.
/// Measured at well under 2s on an M4; the margin is for a loaded machine.
const STARTUP_GRACE: Duration = Duration::from_secs(5);

/// A running `nudge` whose entire filesystem footprint is redirected into a
/// temp dir, and which is killed when the test ends.
///
/// The redirection matters more on macOS than it did on Windows: `dirs`
/// resolves `<Documents>/Nudge/` from `$HOME`, so an un-sandboxed run would
/// append to the developer's real journal. Overriding `HOME` moves the config,
/// the journal, and any future app-state file in one shot — verified: a 12s run
/// creates `$HOME/Documents/Nudge/config.json` and touches nothing outside it.
struct SandboxedApp {
    child: Child,
    home: tempfile::TempDir,
}

impl Drop for SandboxedApp {
    fn drop(&mut self) {
        // Best-effort: a leaked always-on-top window would poison every
        // subsequent test in the same run.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl SandboxedApp {
    fn launch() -> Self {
        let home = tempfile::tempdir().expect("temp HOME");
        std::fs::create_dir_all(home.path().join("Documents")).expect("Documents");

        let child = Command::new(binary_path())
            .env("HOME", home.path())
            .spawn()
            .expect("spawn nudge — run `cargo build --bin nudge` first");

        std::thread::sleep(STARTUP_GRACE);
        Self { child, home }
    }

    /// Like `launch`, but seeds `config.json` with the given interval
    /// *before* the binary is spawned, instead of letting the app write its
    /// own first-run default. Uses the exact schema the app itself writes:
    /// `{"hotkey": "Ctrl+Shift+Space", "default_interval_minutes": ..., "autostart": false}`.
    /// Pre-seeding avoids a race with the app's own first-run write, and is
    /// what lets a 3s interval (0.05 min) drive the reappearance test instead
    /// of waiting out a real-world default.
    fn launch_with_interval(minutes: f64) -> Self {
        let home = tempfile::tempdir().expect("temp HOME");
        let config_dir = home.path().join("Documents/Nudge");
        std::fs::create_dir_all(&config_dir).expect("Documents/Nudge");
        std::fs::write(
            config_dir.join("config.json"),
            format!(
                r#"{{"hotkey": "Ctrl+Shift+Space", "default_interval_minutes": {minutes}, "autostart": false}}"#
            ),
        )
        .expect("seed config.json");

        let child = Command::new(binary_path())
            .env("HOME", home.path())
            .spawn()
            .expect("spawn nudge — run `cargo build --bin nudge` first");

        std::thread::sleep(STARTUP_GRACE);
        Self { child, home }
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn config_path(&self) -> PathBuf {
        self.home.path().join("Documents/Nudge/config.json")
    }

    fn journal_path(&self) -> PathBuf {
        self.home.path().join("Documents/Nudge/journal-rust.ndjson")
    }
}

/// TODO: resolve through `CARGO_BIN_EXE_nudge` once these tests run in CI.
/// Hardcoding the debug path keeps the first iteration runnable by hand.
fn binary_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/nudge")
}

/// Screen-space rect, top-left origin, in points (not backing pixels).
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
}

/// Geometry plus the `kCGWindowNumber` of one on-screen window, as reported
/// by `CGWindowListCopyWindowInfo`. The id is what `screencapture -l` needs
/// to grab the window alone instead of a screen region.
#[derive(Debug, Clone, Copy)]
struct WindowInfo {
    /// What `capture_window` passes to `screencapture -l`, which is what makes
    /// the appearance assertions independent of the wallpaper behind the
    /// window.
    id: u32,
    rect: Rect,
    /// `kCGWindowIsOnscreen` — whether the window is actually presented right
    /// now, as opposed to merely existing. `window_rect` is built on
    /// `kCGWindowListOptionAll`, which keeps listing a window the app has
    /// ordered out (e.g. dismissed but not destroyed); `rect` being `Some`
    /// only proves the window object still exists, not that anyone can see
    /// it. `window_rect(pid).is_some()` is therefore NOT a visibility test —
    /// use `popup_visible` for that.
    on_screen: bool,
}

/// Window geometry via `CGWindowListCopyWindowInfo`, filtered by owner pid.
///
/// Three reasons this beats System Events: it drops the Accessibility grant
/// requirement entirely, it sees every window the process owns (System
/// Events reports only the frontmost window per process), and it yields the
/// window id that `screencapture -l` needs to grab the window alone instead
/// of a screen region.
///
/// Not an option: finding widgets by role or label. `accesskit_macos` is
/// linked into the binary, but the running app exposes an `AXWindow` with no
/// children — `entire contents` comes back empty — so there is nothing below
/// the window to query. Any assertion about fields has to be made on pixels.
fn window_rect(pid: u32) -> Option<WindowInfo> {
    // `kCGWindowListOptionAll` (0), deliberately not
    // `kCGWindowListOptionOnScreenOnly`. The whole point of this harness is
    // catching a window the OS has placed off-screen — that is exactly the
    // property OnScreenOnly filters on, so using it here would silently drop
    // the one window this function exists to find and turn a real placement
    // bug into a spurious "no nudge window found".
    let windows = CGDisplay::window_list_info(kCGWindowListOptionAll, None)?;

    let owner_pid_key = unsafe { CFString::wrap_under_get_rule(kCGWindowOwnerPID) };
    let bounds_key = unsafe { CFString::wrap_under_get_rule(kCGWindowBounds) };
    let number_key = unsafe { CFString::wrap_under_get_rule(kCGWindowNumber) };
    let on_screen_key = unsafe { CFString::wrap_under_get_rule(kCGWindowIsOnscreen) };

    // Tracks the best candidate found so far, alongside whether it is on an
    // elevated (always-on-top) window layer — see the comment below on why
    // that beats picking by area alone.
    let mut best: Option<(WindowInfo, bool)> = None;

    for i in 0..windows.len() {
        let Some(entry) = windows.get(i) else {
            continue;
        };
        // Each element is a CFDictionaryRef describing one window; the array
        // itself only carries opaque pointers.
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
            // A process can own zero-size bookkeeping surfaces (e.g. the
            // accessibility bridge's own windows); never the popup itself.
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

        // `nudge`'s popup is built with `.with_always_on_top()` (src/main.rs),
        // which macOS represents as a nonzero CGWindowLevel — surfaced here as
        // `kCGWindowLayer`. This is the reliable signal, confirmed empirically:
        // the same pid also owns an unnamed 500x500 layer-0 window (an
        // AppKit/winit-internal surface, not the popup) that is *larger* than
        // the real 520x320 popup, so ranking candidates by area alone picks
        // the wrong one. Elevated-layer candidates win outright; only among
        // ties (or if none are elevated) does area break the tie.
        let layer_key = unsafe { CFString::wrap_under_get_rule(kCGWindowLayer) };
        let elevated = dict
            .find(&layer_key)
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|n| n.to_i64())
            .is_some_and(|layer| layer != 0);
        let area = rect.w * rect.h;
        let candidate = (
            WindowInfo {
                id,
                rect,
                on_screen,
            },
            elevated,
        );

        let replace = match best {
            None => true,
            Some((ref b, b_elevated)) => match (elevated, b_elevated) {
                (true, false) => true,
                (false, true) => false,
                _ => area > b.rect.w * b.rect.h,
            },
        };
        if replace {
            best = Some(candidate);
        }
    }

    best.map(|(window, _elevated)| window)
}

/// Whether the popup is actually shown to the user right now — not merely
/// whether a window object for it still exists. `window_rect(pid).is_some()`
/// alone is NOT this test: it is built on `kCGWindowListOptionAll`, which
/// keeps reporting a window the app has ordered off screen (e.g. right after
/// dismissal). Visibility means both a rect was found and its
/// `kCGWindowIsOnscreen` flag is set.
fn popup_visible(pid: u32) -> bool {
    window_rect(pid).is_some_and(|w| w.on_screen)
}

/// Synthesise the Esc keystroke that dismisses the popup — the macOS
/// analogue of the synthetic Esc `tests/perf_idle.rs` sends via `SendInput`
/// on Windows.
///
/// System Events is used here ONLY to deliver a keystroke, never to read
/// window geometry: its AX origin disagrees with Core Graphics for this same
/// window (System Events reports this window at y=-1137/-1160, while
/// `CGWindowListCopyWindowInfo` reports a stable, centred (475, 162) — see
/// `window_is_within_the_visible_screen`). Any geometry assertion in this
/// module must go through `window_rect`, not through System Events.
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

    // key code 53 = Esc.
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

/// Bounds of the main display, in the same top-left origin space as
/// `window_rect`.
///
/// Deliberately the whole display rather than `NSScreen.visibleFrame`: a window
/// tucked under the Dock is still on screen, and mixing the two APIs would mean
/// converting between AppKit's bottom-left origin and this one — the exact
/// arithmetic that makes off-screen bugs hard to see.
fn main_display_frame() -> Rect {
    let bounds = CGDisplay::main().bounds();
    Rect {
        x: bounds.origin.x,
        y: bounds.origin.y,
        w: bounds.size.width,
        h: bounds.size.height,
    }
}

/// Capture one window on its own, alpha channel intact.
///
/// `-l <windowID>` grabs the window's own output instead of a screen region,
/// so the frame carries what the app painted and nothing of what is behind
/// it. Every assertion about the card *itself* has to be made on this rather
/// than on a region grab: over a region, "the card has no contour" and "the
/// card has a contour the wallpaper happens to match" are the same picture.
/// This is the property the Windows harness gets for free by grabbing the
/// HWND.
///
/// `-o` omits the window shadow AppKit would otherwise composite into the
/// frame, which widens every edge and moves the measured card bounds.
///
/// The inverse — what the *user* sees, the app blended with a real backdrop —
/// is `capture_region` over `examples/control_backdrop.rs`.
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

/// The painted card's bounds inside a window capture, as pixel indices.
///
/// Found on the alpha channel, which is unambiguous here: the popup clears its
/// framebuffer to a fully transparent black (`CLEAR_COLOR_TRANSPARENT`), so
/// everything the app did not paint is alpha 0. The 128 threshold also
/// excludes the card's own drop shadow, which peaks well under half opacity
/// just outside the card edge.
fn card_bbox(img: &image::RgbaImage) -> (u32, u32, u32, u32) {
    let (w, h) = img.dimensions();
    let solid = |x: u32, y: u32| img.get_pixel(x, y).0[3] > 128;

    let mid_y = h / 2;
    let mid_x = w / 2;
    let xs: Vec<u32> = (0..w).filter(|&x| solid(x, mid_y)).collect();
    let ys: Vec<u32> = (0..h).filter(|&y| solid(mid_x, y)).collect();
    assert!(
        !xs.is_empty() && !ys.is_empty(),
        "no painted pixels across the middle of the {w}x{h} window capture — \
         the popup rendered nothing"
    );
    (xs[0], ys[0], *xs.last().unwrap(), *ys.last().unwrap())
}

/// Luminance of a captured pixel composited over black.
///
/// Black is the adversarial backdrop on purpose: it is where a dark card's
/// shadow contributes nothing, so anything still visible here is drawn by the
/// app rather than borrowed from what is behind the window.
fn luma_over_black(img: &image::RgbaImage, x: u32, y: u32) -> f64 {
    let [r, g, b, a] = img.get_pixel(x, y).0;
    let a = f64::from(a) / 255.0;
    a * (0.299 * f64::from(r) + 0.587 * f64::from(g) + 0.114 * f64::from(b))
}

/// Capture a screen region to PNG. What the user sees: the app composited with
/// whatever is behind the window. For the card in isolation use
/// `capture_window`.
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

// ---------------------------------------------------------------------------
// Control: does the harness itself work?
// ---------------------------------------------------------------------------

/// Expected GREEN today. Nothing below is trustworthy until this passes — a red
/// test means "the port is incomplete" only if the rig can sandbox a launch and
/// take a picture at all.
///
/// It asserts the two halves separately on purpose: config creation proves the
/// app really started under the redirected HOME (and that the redirection took,
/// so the developer's own journal is safe), and the capture proves the TCC
/// grant is live in *this* process tree.
#[test]
#[ignore = "needs a GUI session + Screen Recording grant; run via --ignored"]
fn harness_can_sandbox_and_capture() {
    let app = SandboxedApp::launch();

    assert!(
        app.config_path().exists(),
        "app did not write {} — it never got far enough to be worth testing",
        app.config_path().display()
    );

    let shot = app.home.path().join("display.png");
    capture_region(main_display_frame(), &shot);

    let img = image::open(&shot).expect("captured file is not a decodable image");
    assert!(
        img.width() > 0 && img.height() > 0,
        "captured an empty image"
    );
}

// ---------------------------------------------------------------------------
// Acceptance criteria for the port
// ---------------------------------------------------------------------------

/// The spotlight lands where spec §1 says, in points.
///
/// The unit test for `window_position` cannot see the failure this catches.
/// `CGWindowListCopyWindowInfo` reports points, `ViewportCommand::OuterPosition`
/// takes points, but the *pixels* of this display are 2940x1912 — so a
/// placement computed the Windows way (`GetSystemMetrics` returns physical
/// pixels) would put the card at (-235, 478) and still pass a
/// pure-arithmetic test. Only a measurement of the real window separates the
/// two.
///
/// Worth knowing before trusting any other geometry here: the AX path
/// disagrees. System Events reports this same window at y=-1137/-1160 — with a
/// correct 520x320 size, which is what makes the reading convincing — and that
/// negative origin is an artifact of the AX coordinate source, not the app.
/// Anything that measures placement must go through `CGWindowListCopyWindowInfo`.
///
/// This check also has to run before any capture-based assertion: those pass
/// silently on an empty crop when the window is not on screen.
#[test]
#[ignore = "needs a GUI session; run via --ignored"]
fn window_is_within_the_visible_screen() {
    let app = SandboxedApp::launch();
    let window = window_rect(app.pid()).expect("no nudge window found");
    let display = main_display_frame();
    eprintln!("window={:?} display={display:?}", window.rect);

    assert!(
        window.rect.intersects(&display),
        "popup is off-screen at {:?} — invisible to the user despite a live process",
        window.rect
    );

    use nudge_web::nudge_state::{POPUP_SIZE, window_position};
    let want = window_position((display.w as u32, display.h as u32), POPUP_SIZE);
    assert_eq!(
        (window.rect.x as i32, window.rect.y as i32),
        want,
        "spec §1 wants the card horizontally centred with its top at 25% of the \
         {}x{}-point display. Feeding the same arithmetic this display's *pixels* \
         instead would land it at {:?} — which is the mistake this assertion exists \
         to catch, so check the units before adjusting the expectation.",
        display.w,
        display.h,
        window_position(
            (
                CGDisplay::main().pixels_wide() as u32,
                CGDisplay::main().pixels_high() as u32,
            ),
            POPUP_SIZE,
        ),
    );
}

/// RED on the current tree.
///
/// `Timer::is_expired` is compiled only under `cfg(any(test, wasm32))`, and the
/// sole native expiry path lives in the Windows tray thread. On macOS the popup
/// shows once at launch and never again, which makes the app inert — it cannot
/// nudge. This is the highest-value single fix in the port.
///
/// Structured so a failure is attributable to one step: the first two
/// assertions are harness/input sanity checks, and only the third is the
/// actual port defect. Verified by hand before writing this: on the current
/// tree the first two pass and the popup stays gone for 12+ seconds across
/// 4+ intervals.
#[test]
#[ignore = "needs a GUI session + Accessibility grant (System Events synthesises the Esc \
            keystroke); run via --ignored"]
fn popup_reappears_after_the_interval() {
    // 0.05 min = 3s — short enough to observe several intervals without a
    // long-running test, using the exact config schema the app itself writes.
    const INTERVAL: Duration = Duration::from_secs(3);
    const INTERVALS_TO_WAIT: u32 = 4;

    let app = SandboxedApp::launch_with_interval(0.05);

    assert!(
        popup_visible(app.pid()),
        "startup popup never appeared — harness problem, not a port problem"
    );

    dismiss_with_escape();
    // Give the app a moment to process the keystroke and hide the window
    // before asserting it is gone.
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

/// The lowest `kCGWindowLayer` a menu-bar status item is drawn at. The menu
/// bar itself and everything docked into it live at 20 and above; ordinary
/// windows are at 0 and the spotlight popup, always-on-top, is at 3.
const STATUS_ITEM_MIN_LAYER: i64 = 20;

/// One on-screen window at the status-item layer or above.
#[derive(Debug, Clone, Copy, PartialEq)]
struct MenuBarWindow {
    id: u32,
    owner_pid: u32,
    layer: i64,
    rect: Rect,
}

/// Every on-screen window in the menu-bar strip.
///
/// Deliberately unfiltered by pid, because a status item is NOT owned by the
/// process that creates it: on macOS 26 the menu bar is hosted by Control
/// Center, and `CGWindowListCopyWindowInfo` attributes our item to *that* pid.
/// Looking for a tray window under nudge's own pid finds nothing, ever — which
/// is the trap this helper exists to keep the test out of. What is observable
/// is the *set*: adding an item adds a window here and shifts its neighbours.
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

/// The status item exists while the app runs, and only while it runs.
///
/// Needs no synthetic input and no Accessibility grant — the window list is
/// readable by anyone — which makes this the one status-item assertion that
/// can be made without driving the GUI. What it does NOT cover: that a left
/// click summons the popup, and that a right click opens the Show / Settings /
/// Quit menu. Those need real clicks on a real, unlocked screen.
///
/// Three snapshots rather than one, because "a window exists in the menu bar"
/// is true of any Mac at rest. Only the *difference* across the app's lifetime
/// is attributable to nudge, and requiring the new window to disappear again
/// on quit is what rules out an unrelated item (a Wi-Fi popover, a clock tick)
/// having appeared in the same second.
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
    // Under --nocapture this is what tells you *whose* pid hosts the item —
    // Control Center's, not nudge's — and is the first thing to look at when
    // the assertion below goes red.
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
    // Only one of the additions has to be ours, and transient neighbours do
    // turn up in this strip: a cursor resting over the menu bar adds a tooltip
    // popover at a higher layer, and any other app's popover lands here too.
    // Asserting the status-item shape on *every* addition therefore reds the
    // test for a passing mouse rather than for a missing status item.
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
    // Control Center re-lays the strip out after the owner dies; give it a
    // beat before reading the third snapshot.
    std::thread::sleep(Duration::from_secs(2));
    let after = menu_bar_windows();

    // Only the windows identified as status items above are held to this: a
    // transient popover that came and went proves nothing either way.
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

/// The last one to become reachable: it needs synthetic keyboard input, which
/// requires the *PostEvent* grant on top of the two in the module header.
///
/// This is the only test here that exercises the whole product loop rather than
/// one capability — reveal, focus, type, submit, persist — and the only one
/// whose readback is the artifact the user actually keeps.
///
/// Typing is the assertion that matters. Under
/// `ActivationPolicy::Accessory` an app that appears on top but never becomes
/// key still receives Esc and Return (winit maps those from the keycode) while
/// silently dropping every character, because characters only reach egui via
/// AppKit's `interpretKeyEvents:` → `insertText:`. A test that pressed Return
/// alone would pass against exactly that broken app, so the journal line has to
/// carry the typed text back: `doing == TYPED` is what separates "the popup has
/// the keyboard" from "the popup has the key *codes*".
#[test]
#[ignore = "needs a GUI session + PostEvent grant; run via --ignored"]
fn submitting_appends_one_journal_line() {
    /// Something no placeholder or default could produce, so a matching journal
    /// line can only have come from these keystrokes.
    const TYPED: &str = "cgevent submit probe";

    let app = SandboxedApp::launch();
    assert!(
        popup_visible(app.pid()),
        "the launch popup never appeared — nothing to type into, so a red here is a \
         harness problem rather than a submit-path problem"
    );
    assert!(
        !app.journal_path().exists(),
        "the sandbox HOME already has a journal at {} before anything was typed",
        app.journal_path().display()
    );

    // Field 1 (`doing`) holds focus from the first frame — `draw_form` calls
    // `request_focus()` on it — so the text needs no Tab to get there.
    type_text(TYPED);
    press_return();

    // Submit hides the popup, and the journal write happens on the same close
    // path, so the window going off screen is the signal that the write either
    // happened or failed. Poll rather than sleep a fixed time: the write is a
    // few hundred microseconds, but the frame that runs it waits on eframe.
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

// ---------------------------------------------------------------------------
// Appearance (spec §2-3)
// ---------------------------------------------------------------------------
//
// These read `capture_window`, never a region grab. A region grab mixes the
// app's output with the desktop, and mixing is precisely what hides this class
// of defect: over the smooth wallpaper a developer happens to be running, a
// missing contour and a present one differ by nothing measurable. The window
// capture carries the app's own alpha, so the tests can composite it over the
// backdrop that is hardest for the design — black — and ask what survives.

/// Points to pixels for this capture. Not a constant: the same code runs on a
/// 2x internal display and on a 1x external one, and every offset below is
/// stated in the points the spec uses.
fn capture_scale(img: &image::RgbaImage, window: &WindowInfo) -> f64 {
    f64::from(img.width()) / window.rect.w
}

/// The popup's own pixels, with the environment checked first.
///
/// The visibility check is not defensive noise. A `-l` capture of a window the
/// app has ordered out succeeds and returns a correctly-sized, entirely
/// transparent frame, so without it the commonest environment fault — a
/// leftover `nudge` from an earlier run, which suppresses the launch popup of
/// every instance after it (see the module header) — arrives at the assertions
/// disguised as "the card rendered nothing".
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

/// Spec §2: the card's contour is what separates it from the desktop when the
/// shadow cannot.
///
/// The shadow is dark, so over a dark window behind the popup — a maximised
/// terminal, a dark-theme chat client — it contributes nothing and the card's
/// boundary is whatever the fill alone provides, which against black is an 8%
/// luminance step. This asserts the boundary survives that case: composited
/// over black, a light hairline must run the card's straight edges.
///
/// Deliberately compared against the fill *at the same y* rather than against
/// a fixed number. The rows differ in brightness — the focused row carries a
/// tint — so a global threshold would either pass a contourless focused row or
/// fail a correct plain one.
#[test]
#[ignore = "needs a GUI session + Screen Recording grant; run via --ignored"]
fn card_contour_survives_a_dark_backdrop() {
    let app = SandboxedApp::launch();
    let (window, img) = capture_card(&app);
    let scale = capture_scale(&img, &window);

    let (x0, y0, x1, y1) = card_bbox(&img);
    let pt = |v: f64| (v * scale).round() as u32;
    // One point of stroke, then a clear sample of the fill past its antialias.
    let rim_px = pt(1.0).max(1);
    let fill_probe = rim_px + pt(3.0);
    // Skip the rounded corners: there the "edge" is a diagonal and a
    // horizontal probe crosses both stroke and background.
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

/// Spec §3: the focused row is marked by a change of row *background*, with no
/// outline of its own.
///
/// The failure this exists to catch is a highlight drawn as an inset plate:
/// visually it is a second rounded rectangle nested inside the card, and
/// because it cannot stay concentric with the card's corners it reads as a
/// smudged double outline at the top of the card. Insetting is invisible to
/// any assertion about *whether* a highlight exists, so the measurement has to
/// be where the highlight ends horizontally, not whether it is there.
///
/// Three steps, so a red test says which one broke: that a highlight exists at
/// all in the row that opens focused, then that it reaches the card edge
/// sideways, then that it reaches its divider downwards. The two directions
/// fail for different reasons and neither implies the other.
#[test]
#[ignore = "needs a GUI session + Screen Recording grant; run via --ignored"]
fn focused_row_background_fills_its_row() {
    let app = SandboxedApp::launch();
    let (window, img) = capture_card(&app);
    let scale = capture_scale(&img, &window);

    let (x0, y0, x1, _) = card_bbox(&img);
    let pt = |v: f64| (v * scale).round() as u32;
    // Row 1 opens focused (spec §3); row 2 is the unfocused reference. Rows are
    // ROW_HEIGHT tall with a 1pt divider, sampled at their vertical middles.
    let focused_y = y0 + pt(20.0);
    let plain_y = y0 + pt(61.0);
    let middle_x = (x0 + x1) / 2;
    // Just inside the contour — well within the inset an inset plate would
    // leave blank, and clear of the stroke's own antialias.
    let edge_x = x0 + pt(1.0).max(1) + pt(2.0);

    let at_middle =
        luma_over_black(&img, middle_x, focused_y) - luma_over_black(&img, middle_x, plain_y);
    assert!(
        at_middle >= 8.0,
        "the focused row is not tinted at all (Δ{at_middle:.1} against the row below, \
         need 8) — focus is invisible, so the edge measurement below would be \
         meaningless"
    );

    let at_edge = luma_over_black(&img, edge_x, focused_y) - luma_over_black(&img, edge_x, plain_y);
    assert!(
        at_edge >= 8.0,
        "the focused row is tinted in its middle (Δ{at_middle:.1}) but not at the card \
         edge (Δ{at_edge:.1}) — the highlight is an inset plate, not a row background. \
         Its own corners cannot stay concentric with the card's, which is what shows \
         up as a double outline. See spec §3 and `row_field` in src/app.rs."
    );

    // Vertically, the same question. A row is bounded by the card above it and
    // by the divider below, and the tint has to meet both — a tint that stops
    // short leaves a strip of bare card between the highlight and the line,
    // and the highlight reads as sitting crooked in its own row.
    //
    // The divider is found rather than assumed: it is the brightest row in the
    // band under the focused one, and where it lands depends on the field's
    // font metrics, not on the row height the code asks for.
    let scan_from = focused_y;
    let scan_to = y0 + pt(70.0);
    let (divider_y, _) = (scan_from..scan_to)
        .map(|y| (y, luma_over_black(&img, middle_x, y)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .expect("empty scan band");
    let just_above_divider = luma_over_black(&img, middle_x, divider_y - pt(1.5).max(1))
        - luma_over_black(&img, middle_x, plain_y);
    assert!(
        just_above_divider >= 8.0,
        "the focused row is tinted at its top (Δ{at_middle:.1}) but not just above its \
         divider (Δ{just_above_divider:.1}) — the tint is shorter than the row. The row is \
         taller than the height `row_field` allocates for it, because the field's own \
         margins overflow it and `put` advances the cursor by the larger amount; paint the \
         tint on the band the cursor actually moved through, not on the allocated rect."
    );
}

/// A full-screen control pattern from `examples/control_backdrop.rs`, put up
/// behind the popup and taken down when the test ends.
struct Backdrop(Child);

impl Drop for Backdrop {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

impl Backdrop {
    /// `pattern` is `checker` or `dark`. Launched *before* the app under test:
    /// it takes no focus, but the popup is always-on-top too and wins the tie
    /// within that level only by opening later.
    ///
    /// On a developer's own Mac this is the piece that fails: a full-screen
    /// window belonging to any app lives in a Space of its own, and a backdrop
    /// on the default Space is then simply not on the screen being
    /// photographed — no window level can help. The guest runner
    /// (`scripts/native-tests-macos.sh`) has a bare desktop and no Spaces,
    /// which is why it is the reproducible path rather than the fallback.
    /// Either way the caller checks the pattern is really there before
    /// believing anything measured over it.
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

/// Mean high-spatial-frequency energy of a region, as mean |Laplacian|.
///
/// This is the measurement that tells a blur from a dimming. Alpha scales a
/// pattern's contrast but leaves its edges exactly where they were, so it
/// lowers this in proportion; a blur removes the edges, and it collapses.
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

/// One run of the popup over one control pattern, measured where the card
/// actually is.
///
/// The card is located from the app's own alpha channel (`capture_window` →
/// `card_bbox`) and only then looked up in the on-screen capture. Deriving it
/// from assumed geometry instead is how a measurement ends up reading the
/// desktop and reporting it as the card: the popup follows the *main* display,
/// which on a machine with a second monitor attached is not necessarily the one
/// a hardcoded region belongs to.
///
/// Returns `(card mean, card high-frequency energy, backdrop high-frequency
/// energy, backdrop mean)` — the last two are the instrument's own reading,
/// which the caller must check before believing the first two.
fn measure_over(pattern: &str) -> (f64, f64, f64, f64) {
    let _backdrop = Backdrop::up(pattern);
    let app = SandboxedApp::launch();
    let (window, own) = capture_card(&app);

    // Where the card sits inside the window, in points.
    let own_scale = capture_scale(&own, &window);
    let (cx0, cy0, cx1, cy1) = card_bbox(&own);
    let card = Rect {
        x: window.rect.x + f64::from(cx0) / own_scale,
        y: window.rect.y + f64::from(cy0) / own_scale,
        w: f64::from(cx1 - cx0) / own_scale,
        h: f64::from(cy1 - cy0) / own_scale,
    };

    // Grab the card and a strip of untouched backdrop beside it in ONE frame,
    // so the two readings cannot disagree about when they were taken.
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

    // Inside: the middle row, clear of the divider lines and of the text.
    let inside = (
        px(MARGIN + card.w * 0.45),
        px(MARGIN + card.h * 0.42),
        px(MARGIN + card.w * 0.95),
        px(MARGIN + card.h * 0.58),
    );
    // Outside: the margin above the card, which is backdrop and nothing else.
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

/// Spec §2: the card is a frosted surface — what is behind the window reaches
/// it, and reaches it blurred.
///
/// Two runs, because either half alone is satisfied by a wrong render. Over
/// the checkerboard, an *opaque* card destroys the pattern just as completely
/// as a blurred one does, so "the detail is gone" is not evidence of frosting.
/// Against a flat dark field, a card that merely dims its backdrop responds in
/// tone just as a frosted one does, so "the tone tracks the backdrop" is not
/// evidence of blurring either. Only together do they exclude both.
///
/// The thresholds come from measurements on this tree, printed by the test:
/// with the fill alone and no backdrop panel, the checker retained ~0.10 of its
/// energy behind the card; with `platform::sync_card_backdrop` in place it
/// measures 0.00.
#[test]
#[ignore = "needs a GUI session + Screen Recording grant; run via --ignored"]
fn card_is_frosted_glass_not_a_tinted_pane() {
    let (bright_mean, bright_hf, control_hf, control_mean) = measure_over("checker");
    eprintln!(
        "[frost] over checker: card mean={bright_mean:.1} hf={bright_hf:.2} | \
         backdrop mean={control_mean:.1} hf={control_hf:.1}"
    );
    assert!(
        control_hf > 20.0 && control_mean > 80.0,
        "the checkerboard is not behind the popup (backdrop reads mean {control_mean:.1}, \
         hf {control_hf:.1}) — nothing below this measures the card, it measures the \
         developer's desktop. Build the instrument first: \
         `cargo build --example control_backdrop`."
    );

    let (dark_mean, _, _, dark_control) = measure_over("dark");
    eprintln!("[frost] over dark: card mean={dark_mean:.1} | backdrop mean={dark_control:.1}");
    assert!(
        dark_control < 40.0,
        "the dark control field is not behind the popup either (reads {dark_control:.1})"
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

/// Spec §2: exactly one shadow.
///
/// The failure this catches is two of them at once. AppKit derives a window
/// shadow from the window's alpha mask, so if the renderer also paints a soft
/// shadow into that alpha, the silhouette AppKit outlines is bigger than the
/// card and its edge lands out in open space — a dark ring detached from the
/// contour, which is what "an outline behind the outline" looks like.
///
/// The measurement is **monotonicity, not steepness**. Measured on this tree,
/// luminance walking outward from the card over a flat backdrop:
///
/// ```text
///   px above card:      1     3     5     7     9    11    13    17    31
///   renderer only:    176   179   182   185   188   191   194   200
///   compositor only:  134   183   189   191   193   194   195   197   200
///   both:             163   168   172   175   132   163   189   196   200
/// ```
///
/// Steepness does not separate them: the compositor's shadow is legitimately
/// tight, dropping 49 levels in the two pixels next to the card, which is more
/// abrupt than anything the broken case does. What only the broken case has is
/// a *reversal* — 175 at 7px, 132 at 9px — light, then dark again, further
/// out. A shadow of any hardness only ever gets lighter with distance.
///
/// A flat bright backdrop, because neither of the other two can show this: a
/// checkerboard's own cell edges are an order of magnitude larger than the
/// signal, and a dark field cannot be darkened by a dark shadow.
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

    // A tall thin strip straight up from the card's top edge. Above the card
    // is where the app's own shadow is weakest (it is offset downwards), so it
    // is where an added rim stands out most.
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

    // One luminance per row, averaged across the strip's width to shrug off
    // dither and sampling noise.
    let profile: Vec<f64> = (0..h).map(|y| mean_luma(&img, 0, y, w, y + 1)).collect();

    // The instrument: the far end of the strip must still be the untouched
    // backdrop. If the app's shadow reaches all the way there, or something
    // else is on screen, the steepness below is measured against nothing.
    let far = profile[0];
    assert!(
        far > 150.0,
        "the top of the strip reads {far:.0}, not the flat light backdrop — either the \
         pattern is not behind the popup or the shadow reaches further than {REACH}pt, \
         and nothing below this measures the shadow's shape"
    );

    // Walk outward from the card and look for the backdrop getting *darker*
    // again. `profile` runs far-to-near, so reversing it walks away from the
    // card, and any fall along the way is a shadow that is not this card's.
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

/// What the judge is asked. Deliberately does not enumerate the things the
/// assertions above already measure: repeating them buys nothing, and a
/// checklist is exactly what stops a judge from reporting the defect nobody
/// thought to look for. It is told what the popup is, what is normal, and to
/// say what is wrong.
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

/// A coarse catch-all: gross visual breakage nobody wrote an assertion for.
///
/// **Its sensitivity was measured, and it is low.** Against two builds
/// differing only in the defect that started this section — a focus highlight
/// inset inside the card, its corners diverging from the card's own — the
/// judge returned "pass" for both, at `detail=low` and at `detail=high`, on
/// `gpt-5.4-mini` and on `gpt-5.5` at high reasoning effort, with a prompt
/// that names "misaligned or doubled outlines" outright. So this is not a
/// substitute for the pixel assertions above, and adding a subtle check here
/// instead of there will silently not run. What it does catch, verified the
/// same way: a card with a strip of backdrop cut through it comes back
/// "pass": false, unprompted and correctly described.
///
/// Kept for that band — a blank card, a missing row, text in a corner, a
/// surface torn open — where the specific assertions have nothing to say and
/// a human would see the problem instantly.
///
/// It is shown the card over the control backdrop rather than over the desktop
/// so its verdict is about the app and not about what the developer had open.
#[test]
#[ignore = "needs a GUI session, Screen Recording, an OPENAI_API_KEY + network; run via --ignored"]
fn card_passes_the_vision_judge() {
    let _backdrop = Backdrop::up("checker");
    let app = SandboxedApp::launch();
    let (window, _) = capture_card(&app);

    // The whole window, so the judge sees the card's edges and the pattern
    // around them — a crop to the card would hide exactly the boundary faults
    // it is being asked about.
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

// ---------------------------------------------------------------------------
// Synthetic keyboard input
// ---------------------------------------------------------------------------

/// Virtual keycode for Return. Sent as a bare keycode with no unicode payload,
/// because egui reads Return from the keycode and would ignore the text.
const KEYCODE_RETURN: CGKeyCode = 36;

/// The `CGEventSource` every synthetic key here is created from.
///
/// The state id makes no observable difference — `Private`, `HIDSystemState`
/// and `CombinedSessionState` were measured in the guest to deliver and to drop
/// characters in exactly the same situations. `CombinedSessionState` is the
/// session-wide one, so it is the honest default rather than a fix for
/// anything.
fn event_source() -> core_graphics::event_source::CGEventSource {
    use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
    CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .expect("CGEventSourceCreate failed — is there a login session?")
}

/// Type `text` one character at a time, as `CGEventPost` key-down/key-up pairs
/// carrying the character in the event's unicode string.
///
/// Per character rather than one event with the whole string: AppKit delivers a
/// key event's unicode payload through a single `insertText:`, so a one-shot
/// event would test a path no keyboard can produce. The pauses are what let
/// eframe run a frame between characters — without them the events pile up in
/// the queue and a dropped one looks like a dropped keystroke.
fn type_text(text: &str) {
    use core_graphics::event::{CGEvent, CGEventTapLocation};

    for ch in text.chars() {
        let mut buf = [0u16; 2];
        let utf16 = ch.encode_utf16(&mut buf);
        for down in [true, false] {
            // Keycode 0 with an explicit unicode string: the character is what
            // the app must receive, and hardcoding a per-character keycode table
            // would only add a second place for the layout to disagree.
            let event = CGEvent::new_keyboard_event(event_source(), 0, down)
                .expect("CGEventCreateKeyboardEvent failed");
            event.set_string_from_utf16_unchecked(utf16);
            event.post(CGEventTapLocation::HID);
            std::thread::sleep(Duration::from_millis(30));
        }
    }
}

/// Press and release Return, the submit gesture.
fn press_return() {
    use core_graphics::event::{CGEvent, CGEventTapLocation};

    for down in [true, false] {
        let event = CGEvent::new_keyboard_event(event_source(), KEYCODE_RETURN, down)
            .expect("CGEventCreateKeyboardEvent failed");
        event.post(CGEventTapLocation::HID);
        std::thread::sleep(Duration::from_millis(60));
    }
}
