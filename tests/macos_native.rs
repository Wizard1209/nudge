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
//! The status item's two gestures — a left click to summon the popup, a
//! right click for its Show / Settings / Quit menu — are covered by
//! `left_clicking_the_status_item_opens_the_popup` and
//! `right_clicking_the_status_item_shows_the_menu`, with real `CGEvent` mouse
//! input: the item is hosted by Control Center, a window this process does
//! not own, and its action is sent on mouse-up events, so a synthetic click
//! is what reaches it.
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

/// Wall-clock allowance for eframe to create and present its first frame.
/// Measured at well under 2s on an M4; the margin is for a loaded machine.
const STARTUP_GRACE: Duration = Duration::from_secs(5);

/// A running `nudge` whose entire filesystem footprint is redirected into a
/// temp dir, and which is killed when the test ends.
///
/// The redirection matters more on macOS than it did on Windows: `dirs`
/// resolves both roots from `$HOME` — `~/Documents/Nudge/` for the journal
/// and `~/Library/Application Support/Nudge/` for the config — so an
/// un-sandboxed run would append to the developer's real journal. Overriding
/// `HOME` moves both, and any future app-state file, in one shot.
struct SandboxedApp {
    child: Child,
    home: tempfile::TempDir,
}

/// Where the app keeps `config.json` on macOS, relative to `$HOME`. Spelled
/// out rather than imported from `nudge::config`: this file is a black-box
/// harness, and a path it derives the same way the app does would agree with
/// the app even when both are wrong.
const CONFIG_SUBDIR: &str = "Library/Application Support/Nudge";

impl Drop for SandboxedApp {
    fn drop(&mut self) {
        // Best-effort: a leaked always-on-top window would poison every
        // subsequent test in the same run.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// The sprite the sprite cases show: a 720×430 px silhouette, every pixel
/// either `(0,255,0,0)` or `(0,255,0,255)`, opaque from (4,6) to (717,424).
/// Embedded rather than read from the tree, because the guest receives only
/// binaries.
const SPRITE_GREEN: &[u8] = include_bytes!("../electron/tests/e2e/fixtures/sprite-green.png");
/// Where `launch_with_config` writes the fixture, relative to `$HOME`.
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
        Self { child, home }
    }

    /// Like `launch`, but seeds `config.json` with the given interval
    /// *before* the binary is spawned, instead of letting the app write its
    /// own first-run default. Pre-seeding avoids a race with the app's own
    /// first-run write, and is what lets a 3s interval (0.05 min) drive the
    /// reappearance test instead of waiting out a real-world default.
    fn launch_with_interval(minutes: f64) -> Self {
        Self::launch_with_config(&format!(
            r#"{{"hotkey": "Ctrl+Shift+Space", "default_interval_minutes": {minutes}, "autostart": false}}"#
        ))
    }

    /// Like `launch`, but with this `config.json` in place before the binary
    /// is spawned. The green sprite fixture is written into HOME first, and
    /// `{sprite}` in `config` stands for its absolute path.
    fn launch_with_config(config: &str) -> Self {
        Self::launch_inner(config, false)
    }

    /// The same, with stderr captured so a test can assert on what the app
    /// logged. Read it with [`SandboxedApp::stderr_after_exit`], which kills
    /// the child first — the pipe is never drained while the process runs.
    fn launch_with_config_logging(config: &str) -> Self {
        Self::launch_inner(config, true)
    }

    /// `nudge --settings`: the settings window alone, no popup process beside
    /// it. HOME is redirected like `launch`, so the config it shows and saves
    /// is the sandbox's.
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

    /// Kill the child and return everything it wrote to stderr. Only valid on
    /// an app made by [`SandboxedApp::launch_with_config_logging`].
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

/// TODO: resolve through `CARGO_BIN_EXE_nudge` once these tests run in CI.
/// Hardcoding the debug path keeps the first iteration runnable by hand.
fn binary_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/nudge")
}

/// A `config.json` whose `sprite` block (spec §10) points at the fixture
/// `launch_with_config` writes into HOME.
fn config_with_sprite(enabled: bool, x: i32, y: i32, card_width: u32) -> String {
    format!(
        r#"{{"hotkey": "Ctrl+Shift+Space", "default_interval_minutes": 10, "autostart": false,
  "sprite": {{"enabled": {enabled}, "path": "{{sprite}}", "x": {x}, "y": {y}, "card_width": {card_width}}}}}"#
    )
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

    /// The same rect to within a point on every side.
    fn same_as(&self, other: &Rect) -> bool {
        (self.x - other.x).abs() <= 1.0
            && (self.y - other.y).abs() <= 1.0
            && (self.w - other.w).abs() <= 1.0
            && (self.h - other.h).abs() <= 1.0
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

    let layer_key = unsafe { CFString::wrap_under_get_rule(kCGWindowLayer) };

    // The largest candidate found so far — see the comment below on which
    // windows are candidates.
    let mut best: Option<WindowInfo> = None;

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

        // The popup is identified by its layer, never by its size. It is
        // built with `.with_always_on_top()` (src/main.rs), which macOS
        // represents as a floating CGWindowLevel — surfaced here as
        // `kCGWindowLayer` 3. The same pid owns windows on either side of
        // that, confirmed empirically: below, unnamed layer-0 surfaces (a
        // 500x500 one, AppKit/winit-internal, and one that measures the
        // display in pixels while the popup is up and 2x2 while it is hidden);
        // above, the status item's pulled-down menu, an on-screen window at
        // layer 101 while it is up. Neither is the popup, and both can be
        // larger than it: the hidden popup is parked at one point. So only an
        // elevated layer below the menu-bar ones (`STATUS_ITEM_MIN_LAYER`) is
        // a candidate. Only one window has ever been seen there; area merely
        // keeps the pick deterministic should a second appear.
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
/// `CGWindowListCopyWindowInfo` reports the work area — see
/// `window_covers_the_work_area_and_the_card_sits_at_its_anchor`). Any
/// geometry assertion in this module must go through `window_rect`, not
/// through System Events.
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

/// `NSScreen.mainScreen`'s `visibleFrame` — in AppKit's bottom-left space,
/// as read — and its `backingScaleFactor`.
///
/// Read from AppKit through a JXA one-liner: both are `NSScreen` facts with
/// no Core Graphics twin (`CGDisplayPixelsWide` reports the mode's logical
/// size, 1024 on the guest's 2x display), and JXA reaches AppKit from any
/// thread.
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

/// The main display's work area — what the menu bar and the Dock leave free
/// — in the same top-left space as `window_rect`. This is the window while
/// the popup is visible (spec §1). AppKit's origin is bottom-left, so the
/// rect is flipped with the display's height — the one conversion this file
/// otherwise avoids, done in one place.
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

/// Backing pixels per point of the main display: 2 on a Retina panel. A
/// sprite is drawn at 1 PNG pixel = 1 device pixel (spec §10), so this is
/// what turns the fixture's pixel size into points.
fn display_scale() -> f64 {
    main_screen_facts().1
}

/// The sprite fixture's size in points on this display.
fn sprite_points() -> (f64, f64) {
    let scale = display_scale();
    (720.0 / scale, 430.0 / scale)
}

/// The fixture decoded, with the bbox of its opaque pixels: what its green
/// looks like before the app draws it, so the capture is compared with the
/// file rather than with numbers copied from it.
fn sprite_fixture() -> (image::RgbaImage, (u32, u32, u32, u32)) {
    let img = image::load_from_memory(SPRITE_GREEN)
        .expect("the embedded fixture decodes")
        .to_rgba8();
    let bbox = bbox_of(&img, is_green).expect("the fixture has opaque pixels");
    (img, bbox)
}

/// Where the fixture's opaque pixels begin, in its own rows.
fn sprite_opaque_top() -> u32 {
    sprite_fixture().1.1
}

/// Where spec §1 puts the card's top edge: a quarter of the monitor's height,
/// floored, plus 80 pt.
fn card_anchor_top(display: &Rect) -> f64 {
    display.y + (display.h / 4.0).floor() + 80.0
}

/// A bbox of capture pixels as a screen rect in points.
fn bbox_rect(window: &WindowInfo, scale: f64, (x0, y0, x1, y1): (u32, u32, u32, u32)) -> Rect {
    Rect {
        x: window.rect.x + f64::from(x0) / scale,
        y: window.rect.y + f64::from(y0) / scale,
        w: f64::from(x1 - x0 + 1) / scale,
        h: f64::from(y1 - y0 + 1) / scale,
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

/// Whether a captured pixel is the sprite fixture's green, with room for the
/// colour management between the GL surface and the PNG on disk. Nothing the
/// card paints — greys, the red error line — comes near it.
fn is_green(p: &image::Rgba<u8>) -> bool {
    let [r, g, b, a] = p.0;
    a > 128 && g >= 200 && r <= 80 && b <= 80
}

/// The smallest pixel box holding every pixel `keep` accepts, as inclusive
/// indices; `None` if it accepts none.
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

/// The painted card's bounds inside a window capture, as pixel indices.
///
/// Found on the alpha channel, which is unambiguous here: the popup clears its
/// framebuffer to a fully transparent black (`CLEAR_COLOR_TRANSPARENT`), so
/// everything the app did not paint is alpha 0. The 128 threshold also
/// excludes the card's own drop shadow, which peaks well under half opacity
/// just outside the card edge. The sprite's green is left out, so a sprite
/// drawn over the card does not widen the answer — as long as it leaves some
/// of every card edge showing, which the sprite cases arrange.
fn card_bbox(img: &image::RgbaImage) -> (u32, u32, u32, u32) {
    let (w, h) = img.dimensions();
    bbox_of(img, |p| p.0[3] > 128 && !is_green(p)).unwrap_or_else(|| {
        panic!("no painted pixels in the {w}x{h} window capture — the popup rendered nothing")
    })
}

/// The sprite's opaque pixels in a window capture, if any are on it.
fn sprite_bbox(img: &image::RgbaImage) -> Option<(u32, u32, u32, u32)> {
    bbox_of(img, is_green)
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
    // Named separately so a config that lands in the old place fails as
    // "wrote to Documents" rather than as "never started".
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

/// A config left in the old place by an earlier version is carried over.
///
/// The unit test for this drives the move directly with two temp dirs. What
/// it cannot show is that the real binary resolves both roots from the same
/// `$HOME` and runs the migration before it reads — a version that resolved
/// the destination but never called the migration would pass it and fail
/// here.
#[test]
#[ignore = "needs a GUI session; run via --ignored"]
fn a_config_left_in_documents_is_carried_over_to_application_support() {
    let home = tempfile::tempdir().expect("temp HOME");
    let legacy = home.path().join("Documents/Nudge/config.json");
    std::fs::create_dir_all(legacy.parent().unwrap()).expect("Documents/Nudge");
    // An interval no default would produce, so the assertion cannot be
    // satisfied by a freshly-seeded file at the destination.
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

// ---------------------------------------------------------------------------
// Acceptance criteria for the port
// ---------------------------------------------------------------------------

/// The window is the work area and the card lands where spec §1 says, in
/// points.
///
/// The unit tests for `shield_layout` cannot see the failure this catches.
/// `CGWindowListCopyWindowInfo` reports points, `ViewportCommand::OuterPosition`
/// takes points, but the *pixels* of a Retina display are twice its points —
/// so a placement computed the Windows way (`GetSystemMetrics` returns
/// physical pixels) would put the card's top twice as far down and still pass
/// a pure-arithmetic test. Only a measurement of the real window separates
/// the two.
///
/// Worth knowing before trusting any other geometry here: the AX path
/// disagrees. System Events has reported this window with a negative origin
/// while Core Graphics reported it on screen — an artifact of the AX
/// coordinate source, not the app. Anything that measures placement must go
/// through `CGWindowListCopyWindowInfo`.
///
/// This check also has to run before any capture-based assertion: those pass
/// silently on an empty crop when the window is not on screen.
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

/// Frames of the app's menu bar extras (status items), as the Accessibility
/// tree reports them: `(x, y, w, h)` in screen points.
///
/// Through System Events rather than `menu_bar_windows`, because that list
/// attributes the item to Control Center's pid and cannot say which of the
/// strip's windows is ours. The process-level `menu bar 2` can: it is the
/// extras bar of exactly this process. Geometry here is sound where it is not
/// for windows (see `dismiss_with_escape`): a status item has no shadow or
/// backing store to be confused about.
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
    // `{position, size}` of N items prints as x1, y1, …, xN, yN, w1, h1, …, wN, hN.
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

/// The daisy fits the menu bar it lives in.
///
/// The bar does not shrink an oversized image: an `NSImage` whose logical
/// size is a bitmap's pixel size (36 pt for the 2x rep) overflows a 24 pt
/// bar above and below and is drawn pixelated. `status_item::status_image`
/// declares the image and each of its reps 18 pt
/// (`STATUS_ICON_POINTS`), which is what this test pins from the outside: a
/// rep added without its `setSize` shows up here as a 30-odd-point item, as
/// an unsized bitmap did in the Electron prototype. Measured: 36x24 at y=3,
/// the 18 pt image plus the bar's own padding.
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

/// The status item's menu, as System Events sees it — its item labels, or
/// `None` when the item has no menu. `menu 1 of menu bar item 1 of menu bar 2`
/// is the extras-bar item's menu. This is NOT a readout of "the menu is
/// showing": a menu the item exposes through AX is listed whether or not it
/// is up. [`open_menu_window`] is the showing signal; this one says what the
/// menu holds.
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

/// The status item's menu while it is up: an on-screen window of nudge's own
/// pid in the menu-bar layers, standing *below* the bar and taller than it.
/// A pulled-down menu is a window the process owns, unlike the item itself
/// (which `menu_bar_windows` attributes to Control Center), so this is the
/// one observation that separates "showing" from "configured".
fn open_menu_window(pid: u32) -> Option<Rect> {
    menu_bar_windows()
        .into_iter()
        .filter(|w| w.owner_pid == pid)
        .map(|w| w.rect)
        .find(|r| r.y >= 20.0 && r.h > 24.0)
}

/// The centre of the one status item, in the top-left-origin screen space
/// `click_at` posts into — the same space System Events reports it in.
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

/// Left click on the daisy summons the popup — the Windows tray gesture and
/// the spec's, unchanged by who owns the status item.
#[test]
#[ignore = "needs a GUI session + PostEvent grant; run via --ignored"]
fn left_clicking_the_status_item_opens_the_popup() {
    let app = SandboxedApp::launch();
    assert!(
        popup_visible(app.pid()),
        "the launch popup never appeared — harness problem, not a status-item problem"
    );
    press_escape();
    assert!(
        wait_until(Duration::from_secs(3), || !popup_visible(app.pid())),
        "Esc did not dismiss the launch popup — synthetic input is not reaching the app"
    );

    let centre = status_item_centre();
    eprintln!("[status-item] left click at {centre:?}");
    click_at(centre);
    assert!(
        wait_until(Duration::from_secs(3), || popup_visible(app.pid())),
        "a left click on the status item did not bring the popup back"
    );

    // Cleanup, not the assertion. A bare Esc posted the instant the window
    // is on screen is lost — the reveal's activation has not landed yet — so
    // this goes the way `popup_reappears_after_the_interval` does, nudge made
    // frontmost first.
    std::thread::sleep(Duration::from_millis(500));
    dismiss_with_escape();
    assert!(
        wait_until(Duration::from_secs(3), || !popup_visible(app.pid())),
        "the popup the click opened did not take Esc"
    );
}

/// Right click on the daisy drops the menu below it, with the three items
/// `nudge_state::TRAY_MENU_LABELS` promises.
#[test]
#[ignore = "needs a GUI session + PostEvent grant; run via --ignored"]
fn right_clicking_the_status_item_shows_the_menu() {
    let app = SandboxedApp::launch();
    assert!(
        popup_visible(app.pid()),
        "the launch popup never appeared — harness problem, not a status-item problem"
    );
    press_escape();
    assert!(
        wait_until(Duration::from_secs(3), || !popup_visible(app.pid())),
        "Esc did not dismiss the launch popup — synthetic input is not reaching the app"
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

    // The menu is attached to the item only for as long as it is up. Were it
    // left attached after closing, every later left click would drop the menu
    // again instead of reaching the item's action, and the popup could never
    // be summoned from the daisy — so the left click is checked here, after
    // a menu has come and gone, not only on a fresh item.
    let centre = status_item_centre();
    eprintln!("[status-item] left click after the menu closed, at {centre:?}");
    click_at(centre);
    assert!(
        wait_until(Duration::from_secs(3), || popup_visible(app.pid())),
        "after the menu closed, a left click on the status item no longer opens the popup — \
         the menu stayed attached to the item"
    );

    // Cleanup, as `left_clicking_the_status_item_opens_the_popup` does it.
    std::thread::sleep(Duration::from_millis(500));
    dismiss_with_escape();
    assert!(
        wait_until(Duration::from_secs(3), || !popup_visible(app.pid())),
        "the popup the click opened did not take Esc"
    );
}

/// The frame of one row of the status item's menu while it is up, in the
/// same top-left-origin screen space as `status_item_frames` — System Events
/// reports a menu row's own geometry, so the click lands on the row rather
/// than on a guessed row height below the menu's top edge.
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

/// Picking `Show Nudge` from the right-click menu opens the popup — the menu
/// item's action reaches the app end to end when the menu is shown this way,
/// and the menu is down again afterwards.
#[test]
#[ignore = "needs a GUI session + PostEvent grant; run via --ignored"]
fn choosing_show_nudge_from_the_status_item_menu_opens_the_popup() {
    let app = SandboxedApp::launch();
    assert!(
        popup_visible(app.pid()),
        "the launch popup never appeared — harness problem, not a status-item problem"
    );
    press_escape();
    assert!(
        wait_until(Duration::from_secs(3), || !popup_visible(app.pid())),
        "Esc did not dismiss the launch popup — synthetic input is not reaching the app"
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

    // Cleanup, as `left_clicking_the_status_item_opens_the_popup` does it.
    std::thread::sleep(Duration::from_millis(500));
    dismiss_with_escape();
    assert!(
        wait_until(Duration::from_secs(3), || !popup_visible(app.pid())),
        "the popup the menu opened did not take Esc"
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

/// Spec §3: the focused row shows a rounded tint around its text — a field
/// inset from the row's sides and from its divider, not a fill of the whole
/// row and not a plate hugging the card's contour.
///
/// Two failures this exists to catch, and they are opposites. A tint that
/// runs to the card edge is a row fill: a different design, and one that
/// reads as a second card edge where it meets the contour's corners. A tint
/// that is inset sideways but reaches down to the divider is the older defect:
/// a plate painted on the row's declared height rather than the height it
/// really takes, sitting crooked in its own row. Whether a tint exists at all
/// says nothing about either, so the measurements are where it ends.
///
/// Three steps, so a red test says which one broke: that a tint exists at all
/// in the row that opens focused, then where it ends sideways, then where it
/// ends downwards.
#[test]
#[ignore = "needs a GUI session + Screen Recording grant; run via --ignored"]
fn focused_row_tint_is_a_field_inside_the_row() {
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
    // Just inside the contour, clear of the stroke's own antialias: bare card
    // under an inset field, tinted under a row fill.
    let gutter_x = x0 + pt(1.0).max(1) + pt(2.0);
    // Past the field's 10pt inset and its corner radius.
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

    // Vertically, the same question. The row ends at its divider, and the
    // field stops 5pt short of it — a strip of bare card the tint must leave.
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

/// What each `examples/control_backdrop.rs` pattern reads as, in the luma
/// `mean_luma` computes: its `DARK` and `LIGHT` are rgb(20) and rgb(200), and
/// the checker averages the two. A precondition compares against these rather
/// than against a loose "bright enough": a system alert parked over the popup
/// (macOS's periodic screen-recording consent panel is one) is bright too, and
/// a loose bound reads it as the backdrop and goes on to measure the alert.
const BACKDROP_DARK_LUMA: f64 = 20.0;
const BACKDROP_LIGHT_LUMA: f64 = 200.0;
const BACKDROP_CHECKER_LUMA: f64 = (BACKDROP_DARK_LUMA + BACKDROP_LIGHT_LUMA) / 2.0;
/// How far a reading may sit from those and still be the pattern.
const BACKDROP_TOLERANCE: f64 = 4.0;

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

    // The instrument: the far quarter of the strip must still be the untouched
    // backdrop, row for row. If the app's shadow reaches all the way there, or
    // something else is on screen, the shape below is measured against nothing.
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

// ---------------------------------------------------------------------------
// Revealing the popup after the desktop has had its way with it
// ---------------------------------------------------------------------------

/// A reveal-desktop sweep must not leave the popup somewhere the next reveal
/// cannot be seen.
///
/// "Click wallpaper to reveal desktop" (the macOS 14+ default) sweeps every
/// window AppKit considers transient off the screen, and a window at any level
/// above normal is transient unless it says otherwise. While the popup is
/// visible its window covers the work area (spec §1), so no click reaches the
/// wallpaper then; the sweep is provoked while the popup is hidden, and shows
/// when the next reveal orders the window back in while the desktop is still
/// revealed: it comes in above the screen. The displacement is the window
/// server's, not the window's — AppKit's frame still reads the spec place —
/// so re-placing the window does not undo it.
///
/// Whether the click really entered reveal mode is checked on a window of
/// another app, not on the popup: the popup's own movement is the thing under
/// test.
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
        "launch popup never appeared — harness problem, not a placement problem"
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
    // A few frames' grace, so a placement the app corrects on its next tick
    // counts as placed.
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

    // Reveal mode is still on (asserted above), so a sweep applied late would
    // show up here.
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

// ---------------------------------------------------------------------------
// Which monitor the popup takes
// ---------------------------------------------------------------------------

/// Spec §1: "Which monitor: the one with the keyboard focus — the display
/// holding the frontmost application's front window, as Spotlight chooses —
/// read when the popup is shown, on every trigger alike. With no such window
/// it is the display under the pointer; the pointer's display on its own is
/// the valid alternative, not the rule."
///
/// A window of another app is made frontmost, the hotkey is pressed, and the
/// popup must come up as the work area of the display that holds that window.
///
/// **Green on one display by construction.** The guest has a single display,
/// so every candidate rule — the front window's, the pointer's, the window's
/// own — names the same screen, and this case cannot tell them apart; a port
/// that ignores the front window entirely passes it. What it pins is the
/// shape of the rule where a second display would be read: a front window of
/// another application existing at the moment of the chord, the popup
/// appearing while that app held the focus, and its window being that
/// display's work area. Which display a second one would win is decided in
/// `src/screen_pick.rs`, whose unit tests carry the multi-display cases.
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
    // The display holding the probe window — the main one, see above.
    let work = work_area();

    let app = SandboxedApp::launch();
    assert!(
        popup_visible(app.pid()),
        "launch popup never appeared — harness problem, not a placement problem"
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
    // A few frames' grace, so a placement the app corrects on its next tick
    // counts as placed.
    std::thread::sleep(Duration::from_millis(500));
    let shown = window_rect(app.pid()).expect("nudge window vanished").rect;
    eprintln!("front window {probe:?} of pid {probe_pid}; popup at {shown:?}; work {work:?}");
    assert!(
        shown.same_as(&work),
        "the popup came up at {shown:?}, not as the work area {work:?} of the display \
         holding the frontmost window {probe:?} (spec §1, which monitor)"
    );
}

/// The owner pid and rect of the first on-screen, ordinary-level, named
/// window of another app: something `bring_frontmost` can put in front and
/// the rule can then read.
fn front_probe_window() -> Option<(u32, Rect)> {
    let own = std::process::id() as i64;
    list_windows()
        .into_iter()
        .find(|w| w.layer == 0 && w.on_screen && w.owner != own && w.named && w.rect.w > 100.0)
        .map(|w| (w.owner as u32, w.rect))
}

/// Make this process the frontmost application, through System Events, the
/// way `dismiss_with_escape` does for `nudge` — by pid rather than by name,
/// since the probe may be any app the guest has up.
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

/// Spec §1: the popup "takes the keyboard when shown even if another
/// application is activating at the same moment, and hands it back to the
/// application that had it when it closes, by any of the three Leave
/// triggers and by Enter alike."
///
/// A window of another app is made frontmost, the hotkey summons the popup,
/// and after it closes that app must be the frontmost one again — once by
/// Esc, once by Enter on a typed answer, the journaled Submit. Both
/// keystrokes are raw, posted to whatever holds the keyboard, so a popup that
/// never took it fails here too. Without the
/// hand-back, nudge stays the active application with no key window: every
/// keystroke goes nowhere until the user clicks, and the next show reads
/// nudge as the frontmost app, finds no window of its own at the ordinary
/// level and falls through to the pointer's display.
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
        "launch popup never appeared — harness problem, not an activation problem"
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
        // The activation lands asynchronously; the popup must hold the
        // keyboard before the raw keystroke below is posted, or the probe
        // takes it.
        assert!(
            wait_until(Duration::from_secs(2), || frontmost_pid()
                == Some(app.pid())),
            "the popup came up but nudge is not the frontmost application (frontmost \
             is {:?}) — it never took the keyboard, so {name} would go to the probe",
            frontmost_pid()
        );
        std::thread::sleep(Duration::from_millis(300));

        // Return on empty fields is the change-interval path (spec §4); an
        // answer makes it the Submit that writes a journal line.
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

/// Spec §1, §4: a click anywhere on the monitor outside the card closes the
/// popup and reaches nothing beneath.
///
/// "Reaches nothing" is read off the desktop: the point is bare wallpaper
/// under the popup's window, and a click that got through would enter
/// reveal-desktop mode and move every other window aside — which the probe
/// window of another app would show.
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
        "launch popup never appeared — harness problem, not a shield problem"
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

// ---------------------------------------------------------------------------
// Sprite overlay (spec §10)
// ---------------------------------------------------------------------------

/// The popup with the green fixture as its sprite, photographed: the window,
/// its capture, the capture's scale, and the card's and the sprite's bboxes
/// in capture pixels. The sprite bbox is `None` when no green is on screen.
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

/// The configured sprite does not change what the window is.
#[test]
#[ignore = "needs a GUI session + Screen Recording grant; run via --ignored"]
fn sprite_window_is_still_the_work_area() {
    let app = SandboxedApp::launch_with_config(&config_with_sprite(true, -41, -277, 679));
    let window = window_rect(app.pid()).expect("no nudge window found");
    let work = work_area();
    eprintln!("window={:?} work={work:?}", window.rect);
    assert!(
        popup_visible(app.pid()),
        "launch popup never appeared with a sprite configured"
    );
    assert!(
        window.rect.same_as(&work),
        "with a sprite the window is {:?}, not the work area {work:?} — spec §10: the \
         window covers the work area; the card and the sprite are drawn inside it",
        window.rect
    );
}

/// Spec §10: the sprite is drawn over the card at its native pixels, and
/// nothing but the card and the sprite's own pixels is opaque.
///
/// The fixture is a silhouette, so "over the card" is checked at one of its
/// opaque pixels — the one nearest the middle of its body — placed 60×40 pt
/// into the card. That leaves every card edge a stretch in the open for
/// `card_bbox`, and the sprite's transparent surround above the card.
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

    // The fixture's opaque pixels are exactly its size less its margins, so
    // their bbox says whether it was drawn 1:1 or resampled — and, since the
    // window starts at the work area's edge, whether it is on the window whole.
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
    // A fixture pixel's place in the capture.
    let at = |(fx, fy): (u32, u32)| (sx0 - fx0 + fx, sy0 - fy0 + fy);

    // Over the card, inside both: the sprite's pixel, not the card's.
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
    // The sprite's own transparent surround, above the card: not painted.
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
    // The card where the sprite is not: the card.
    let bare = shot.img.get_pixel(cx1 - px(40.0), (cy0 + cy1) / 2);
    assert!(
        bare.0[3] > 128 && !is_green(bare),
        "the card beside the sprite reads {bare:?} — the sprite is covering more than \
         its own pixels"
    );
}

/// Spec §10: a sprite that stays inside the work area leaves the card at its
/// §1 anchor.
///
/// The sprite is set wholly to the left of the card, with its top 20 pt below
/// the work area's top; on a narrow display part of it runs off the window,
/// which the spec allows and the card does not notice.
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

/// Spec §10: a sprite that would rise above the work area moves the card down
/// by the overhang, so the sprite is shown whole.
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

/// `enabled: false` with a valid PNG configured: the plain card, no sprite.
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

/// Spec §10: a PNG that fails the check is not a sprite. The running app says
/// why on stderr and shows the plain card.
#[test]
#[ignore = "needs a GUI session + Screen Recording grant; run via --ignored"]
fn sprite_with_a_bad_png_falls_back_and_logs() {
    // An RGB PNG — no alpha channel — is not a sprite by §10.
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

/// Spec §10 "Applying a change": a change edited into `config.json` while the
/// popup is visible waits for the next close; the next reveal shows it.
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

    // Visible: the change must wait. Photographed a few times over the next
    // seconds, the card must stay as it was.
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

/// The faithful launch path for a login item, under the activation contention
/// a user produces by switching apps while it starts.
///
/// Launched by launchd rather than spawned: a launchd job has no parent the
/// window server can attribute the launch to, and that is the case in which
/// the popup has been seen to come up centred and never become key — focus
/// loss cannot close it (it never armed) and Esc goes to whichever app is
/// in front.
#[test]
#[ignore = "needs a GUI session + PostEvent grant; run in the guest via scripts/native-tests-macos.sh"]
fn launchd_start_under_activation_contention_still_yields_a_key_popup() {
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_sampler = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let job = LaunchdApp::bootstrap();

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
    // Who is frontmost, sampled for the whole run — the assertion below only
    // sees the end state, and this says whether nudge was ever given the
    // keyboard on the way there.
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
    // Released however the test ends: a Command held down by a panicking
    // test would be inherited by every test after it.
    let _cmd_up = ReleaseModifiers;

    let pid = job.wait_for_pid(Duration::from_secs(10));
    let appeared = wait_until(Duration::from_secs(10), || popup_visible(pid));
    std::thread::sleep(Duration::from_millis(300));
    stop.store(true, std::sync::atomic::Ordering::SeqCst);
    let presses = switcher.join().expect("switcher thread");
    assert!(
        appeared,
        "the launchd-started popup never appeared; stderr:\n{}",
        job.stderr()
    );
    eprintln!("Cmd-Tab presses during launch: {presses}");

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

/// Whether `a` sits somewhere else than `b`, judged by centre: the order-in
/// and order-out animations scale a window by a few points about its centre.
fn displaced(a: &Rect, b: &Rect) -> bool {
    let centre = |r: &Rect| (r.x + r.w / 2.0, r.y + r.h / 2.0);
    let ((ax, ay), (bx, by)) = (centre(a), centre(b));
    (ax - bx).abs() > 4.0 || (ay - by).abs() > 4.0
}

/// The first on-screen, ordinary-level window of any other app that is at
/// least partly on the display: what the reveal-desktop sweep visibly moves.
fn foreign_probe_window(display: &Rect) -> Option<(u32, Rect)> {
    let own = std::process::id() as i64;
    list_windows()
        .into_iter()
        .find(|w| w.layer == 0 && w.on_screen && w.owner != own && w.named && w.rect.w > 100.0)
        .filter(|w| w.rect.intersects(display))
        .map(|w| (w.id, w.rect))
}

/// The on-screen windows at ordinary-to-floating levels that contain `p`.
/// The Dock's full-display layer and the desktop's own windows are left out:
/// a click there reaches the wallpaper.
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

/// Leaves the desktop as the test found it: a second wallpaper click toggles
/// reveal mode off, and it is only sent if the probe window is still away.
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

/// `nudge` started as a launchd job of the GUI domain, the way the login item
/// starts it, under a label of its own and a throwaway HOME. Booted out and its
/// plist removed when dropped, including on a panic.
struct LaunchdApp {
    label: String,
    domain: String,
    home: tempfile::TempDir,
}

impl LaunchdApp {
    fn bootstrap() -> Self {
        let home = tempfile::tempdir().expect("temp HOME");
        std::fs::create_dir_all(home.path().join("Documents")).expect("Documents");
        let config = home.path().join(CONFIG_SUBDIR).join("config.json");
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
        // A job left behind by an aborted run would make bootstrap fail.
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
        // The plist lives in `home`, which goes with the TempDir.
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

/// The pid of the frontmost application, as Launch Services sees it.
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

/// A left click at a point in the top-left-origin space of `window_rect`,
/// posted as real HID mouse events (a move first, so the cursor is there).
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

/// `click_at` with the right button: the gesture that drops a status item's
/// menu.
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

/// Press and release one key with `modifiers` held, each modifier cap pressed
/// and released as its own flags-changed edge around it. The window server
/// keeps its own idea of what is held, and the flags on a key event never tell
/// it a modifier went up — so a chord posted as one flagged key edge leaves
/// the modifier stuck for every test after this one.
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

/// A bare Esc, posted to whatever has the keyboard — unlike
/// `dismiss_with_escape`, which makes nudge frontmost first and so cannot see
/// a popup that never got the focus.
fn press_escape() {
    press_chord(VK_ESCAPE, CGEventFlags::CGEventFlagNull);
}

/// ⌘Tab as a quick tap: switches to the previous app without lingering long
/// enough to raise the switcher panel.
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

/// Posts "nothing held" on drop.
struct ReleaseModifiers;

impl Drop for ReleaseModifiers {
    fn drop(&mut self) {
        post_flags_changed(VK_COMMAND, CGEventFlags::CGEventFlagNull, 20);
    }
}

/// Spec §10 "Applying a change": a change that arrives while the popup is
/// hidden is applied right there — and the sprite it turns on must be on
/// screen at the next reveal, not only the layout that makes room for it.
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
    // Hidden: the change applies now. Give the watcher a moment to deliver it.
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

// -----------------------------------------------------------------------------
// §9 Window chrome and layout — the settings window.
// -----------------------------------------------------------------------------

/// What §9 sizes the settings window's content to, in points: the form at
/// 560 wide, measured on the native build — the window asks for exactly this
/// (`settings_app::WINDOW_SIZE`), and tests/kittest_card.rs pins that the
/// form adds up to it.
const SETTINGS_CONTENT: (f64, f64) = (560.0, 537.0);

/// The standard title bar's share of a framed window's frame on this macOS,
/// in points: `CGWindowListCopyWindowInfo` reports a 560-point-tall content
/// area as a 592-point frame (measured on macOS 26 for `with_inner_size`).
const TITLE_BAR_HEIGHT: f64 = 32.0;

/// Pixels of a solid fill: a colour, not a grey. Text, hairlines and a
/// neutral button are all below this; an accent fill is well above it.
const FILL_SATURATION: i32 = 60;

/// The settings window of `pid`: the named window, which the process's
/// unnamed bookkeeping surfaces (a 500×500 AppKit one, the menu bar's) are
/// not.
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

/// Makes the process with `pid` frontmost, so posted keystrokes reach it.
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

/// A left-button drag from `from` by `dx` points, as real HID mouse events:
/// the gesture that resizes a resizable window grabbed by its right edge.
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

/// The one solid block of colour in the window's own pixels below the title
/// bar, as a rect in window points: rows on which a run of saturated pixels
/// at least 50 points long stands, at least 8 such rows in a row. A filled
/// button is that; a focus ring, a line of amber text and a grey button are
/// not.
fn filled_block(img: &image::RgbaImage, scale: f64, below: f64) -> Option<Rect> {
    let saturated = |p: &image::Rgba<u8>| {
        let (r, g, b) = (i32::from(p[0]), i32::from(p[1]), i32::from(p[2]));
        r.max(g).max(b) - r.min(g).min(b) >= FILL_SATURATION
    };
    let min_run = (50.0 * scale) as u32;
    let min_rows = (8.0 * scale) as usize;
    let first_row = (below * scale) as u32;

    // Per row, the longest saturated run as (start, end).
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

    // The tallest stretch of consecutive such rows.
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

/// §9 "Window chrome and layout": "The window is sized to the form and is not
/// resizable. The banner has its line above the buttons whether or not it has
/// anything to say, so the window never changes size." And, of the same
/// subsection, "`Cancel`, then `Save`, at the bottom right; `Save` is the one
/// filled button".
///
/// Geometry comes from `CGWindowListCopyWindowInfo` like everywhere else in
/// this file, and "not resizable" is observed the way a user meets it: a drag
/// on the right edge that moves nothing. Nothing is read through System
/// Events: what it reports for this process depends on the Accessibility
/// grant the session happens to hold, so there is no `resizable` attribute
/// to rely on and no button to find by name. The filled button is found in
/// the window's own pixels instead.
#[test]
#[ignore = "needs a GUI session + Accessibility and Screen Recording grants; run via --ignored"]
fn settings_window_is_fixed_size_and_not_resizable() {
    let app = SandboxedApp::launch_settings();
    let window = settings_window(app.pid()).expect("no settings window found");
    eprintln!("settings window={:?}", window.rect);
    assert!(window.on_screen, "the settings window is not on screen");

    // Sized to the form.
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

    // Not resizable: a drag on the right edge, outward, moves nothing.
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

    // Never changes size: a refused save raises the banner — a bare key in the
    // first field, refused with the vocabulary after it, the longest message
    // the banner carries — and the window stays as it was.
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

    // Save — live now, the form differs from the file — is the one filled
    // button, and it sits inside the window, at its bottom right.
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
