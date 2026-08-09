//! macOS backend: the interval timer, run on eframe's main thread.
//!
//! # Why the main thread and not a thread of its own
//!
//! Windows drives the popup from the tray thread's message pump. macOS has no
//! such thread yet, and the AppKit call that hands the revealed window its
//! keyboard focus has to run on the main thread regardless — so the deadline
//! is polled from `App::logic`, which eframe calls on every tick *including*
//! the ticks where the window is hidden and `App::ui` is skipped
//! (`native/epi_integration.rs`: `app.logic(..)` is unconditional, `app.ui(..)`
//! sits behind `if is_visible`). Putting the timer in `ui()` is exactly why
//! the port shipped a popup that appeared once and never again.
//!
//! # Why the repaint request is re-issued on every tick
//!
//! `ctx.request_repaint_after(d)` is per-frame output, not a standing timer:
//! any later frame that does not re-issue it replaces the deadline with
//! infinity and cancels the wake. Issuing it once before hiding was measured
//! to never wake the app at all. Re-issuing the remaining time each tick wakes
//! once, on time, and costs nothing measurable while idle: a hidden app on a
//! 5-minute interval logged two ticks and no change in accumulated CPU time
//! across a 60-second window (`ps -o time`, 10 ms resolution — so under
//! 0.2 ms/s against the project's 25 ms/s ratchet). It is a sleep in the
//! kernel, not a spin: each pass asks for one wake at the deadline and blocks.
//!
//! A background thread calling `ctx.request_repaint()` measures the same and
//! is the fallback if this ever proves fragile; it just has strictly more
//! moving parts (a thread, a join-less shutdown, a `Context` handoff).

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use eframe::egui;
use objc2_app_kit::NSApplication;
use objc2_foundation::MainThreadMarker;

use crate::config;
use crate::hotkey::Hotkey;

/// When the next nudge is due, or `None` when no timer is armed: before the
/// first close (spec §4 freezes the timer until then) and between an expiry
/// and the reset that follows it.
static DEADLINE: Mutex<Option<Instant>> = Mutex::new(None);

/// Edge flags polled by `NudgeApp::ui`, mirroring the Windows ones so the
/// state transition on expiry is the app's existing code path, not a second
/// implementation of it.
static TRAY_CLICKED: AtomicBool = AtomicBool::new(false);
static TIMER_FIRED: AtomicBool = AtomicBool::new(false);

/// Set by the `config_watcher` callback when `config.json` changes on disk,
/// drained by `tick`. See `apply_config_reload` for what "applied" means here
/// today (nothing yet — the live-reloadable field is the hotkey).
static CONFIG_RELOAD_REQUESTED: AtomicBool = AtomicBool::new(false);

/// `true` while the popup is on screen. `tick` parks on it exactly as the
/// tray thread does: while the user is looking at the popup we neither count
/// down nor try to reveal anything. Starts `true` because the app opens with
/// the launch popup.
static POPUP_VISIBLE: AtomicBool = AtomicBool::new(true);

/// Start the OS-integration machinery. Nothing to start yet: the menu-bar
/// status item and the global hotkey are the next slice of the port, and the
/// timer needs no thread of its own (see the module docs).
pub fn start(_hotkey: Option<Hotkey>, _config_path: std::path::PathBuf, _initial: config::Config) {
    // TODO(macos-port): status item + `CGEventTap`/`RegisterEventHotKey`.
}

/// No-op: nothing on this side needs the window handle. The reveal path goes
/// through `NSApplication` and egui's viewport commands, both of which
/// address the app's own main window without one.
pub fn note_window_handle(_handle: isize) {}

pub fn set_tray_clicked() {
    TRAY_CLICKED.store(true, Ordering::SeqCst);
}

pub fn take_tray_clicked() -> bool {
    TRAY_CLICKED.swap(false, Ordering::SeqCst)
}

pub fn take_timer_fired() -> bool {
    TIMER_FIRED.swap(false, Ordering::SeqCst)
}

/// Arm the timer. `duration` is what the Windows tray needs to draw the
/// petal countdown; there is no status item here yet, so only the deadline
/// is kept.
pub fn set_timer_state(deadline: Instant, _duration: Duration) {
    *DEADLINE.lock().unwrap() = Some(deadline);
}

pub fn set_popup_visible(visible: bool) {
    POPUP_VISIBLE.store(visible, Ordering::SeqCst);
}

/// Signal that `config.json` changed on disk. Idempotent — N saves before
/// the next `tick` collapse into one re-read, which is the correct
/// behaviour.
///
/// Unlike the Windows side there is no wake here: `tick` observes the flag on
/// the next frame, which arrives at the timer deadline at the latest. That is
/// good enough only while nothing live-reloadable exists on this platform —
/// the wake lands with the hotkey backend, which is the first thing a reload
/// can actually change mid-run.
pub fn request_config_reload() {
    CONFIG_RELOAD_REQUESTED.store(true, Ordering::SeqCst);
}

/// One pass of the timer, called from `NudgeApp::logic` on every tick.
///
/// Either re-arms the wake for the remaining time, or — once the deadline has
/// passed — raises `TIMER_FIRED` and reveals the window. It deliberately does
/// *not* perform the state transition itself: raising the flag routes expiry
/// into the same `ui()` branch Windows uses, so there is one definition of
/// what a timer-fired open does.
pub fn tick(ctx: &egui::Context) {
    if CONFIG_RELOAD_REQUESTED.swap(false, Ordering::SeqCst) {
        apply_config_reload();
    }

    if POPUP_VISIBLE.load(Ordering::SeqCst) {
        // The popup is up: the countdown is paused and `ui()` is driving its
        // own 1 Hz repaint, so there is nothing to schedule here.
        return;
    }

    let mut slot = DEADLINE.lock().unwrap();
    let Some(deadline) = *slot else {
        return;
    };

    let remaining = deadline.saturating_duration_since(Instant::now());
    if !remaining.is_zero() {
        ctx.request_repaint_after(remaining);
        return;
    }

    // Expired. Disarm so one deadline fires once: every close of a
    // timer-fired popup resets the timer (`nudge_state::decide_close`), and
    // that reset is what re-arms us through `set_timer_state`.
    *slot = None;
    drop(slot);

    TIMER_FIRED.store(true, Ordering::SeqCst);
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    focus_popup(ctx);
    // The window is hidden right now, so `ui()` is being skipped and cannot
    // schedule the frame that will consume `TIMER_FIRED`. Ask for it here.
    ctx.request_repaint();
}

/// Reveal-and-focus, called on every path that puts the popup back on screen.
///
/// `ViewportCommand::Focus` alone is not enough: the window appears, but
/// `viewport().focused` stays false and every keystroke is swallowed — a
/// popup you cannot dismiss with Esc. A background app has to ask AppKit for
/// activation first; the viewport command then lands on an app the window
/// server is willing to give the keyboard to.
///
/// `NSApplication` is main-thread-only, and the marker is the type system
/// saying so. In the app it is always present — eframe calls `logic()` and
/// `ui()` from the event loop on the main thread. The `None` arm is for
/// `tests/kittest_card.rs`, which drives a real `NudgeApp` from a libtest
/// worker thread against no window server at all: there is nothing to
/// activate there, and asking anyway is the unsound path, not the safe one.
pub fn focus_popup(ctx: &egui::Context) {
    if let Some(mtm) = MainThreadMarker::new() {
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    }
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
}

/// Apply a pending config reload.
///
/// Nothing to apply yet, and that is a statement about this platform rather
/// than a stub: of the three config fields only `hotkey` has live effect on a
/// running app (`config_watcher`'s module docs), and macOS has no hotkey
/// registration to re-do. The log line is what makes the watcher observable
/// until then.
fn apply_config_reload() {
    // TODO(macos-port): re-read + diff the file and re-register the hotkey,
    // the way `tray_bridge::apply_config_reload` does.
    eprintln!("[nudge] config.json changed — no live-reloadable macOS state yet");
}
