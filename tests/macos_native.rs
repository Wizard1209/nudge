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
//! All five are green. Each names one capability the port had to restore, and
//! none can be satisfied by making the code merely compile: the harness
//! control, window placement, the interval timer, the menu-bar status item,
//! and journal-writing driven by real keystrokes.
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
    /// Carried for `capture_region`'s outstanding `screencapture -l <windowID>`
    /// TODO — grabbing the window alone is what would make a transparency
    /// assertion independent of the wallpaper behind it.
    #[expect(dead_code, reason = "read once capture_region takes the -l path")]
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

/// Capture a screen region to PNG.
///
/// TODO: prefer `screencapture -l <windowID>` once `window_rect` returns a
/// window id. Capturing the window alone takes the wallpaper out of the frame,
/// which is what lets the transparency assertion be independent of what is
/// behind the window — the property the Windows harness gets for free by
/// grabbing the HWND.
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
    for w in &added {
        assert!(
            w.rect.y >= 0.0 && w.rect.y < 40.0,
            "a status item sits in the menu bar strip at the top of the screen, not at {w:#?}"
        );
    }

    drop(app);
    // Control Center re-lays the strip out after the owner dies; give it a
    // beat before reading the third snapshot.
    std::thread::sleep(Duration::from_secs(2));
    let after = menu_bar_windows();

    let survivors: Vec<&MenuBarWindow> = added
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
