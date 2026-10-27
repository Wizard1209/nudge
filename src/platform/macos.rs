//! macOS backend: the interval timer, the menu-bar status item and the global
//! hotkey, all driven from eframe's main thread.
//!
//! # Why the main thread and not a thread of its own
//!
//! Windows drives the popup from the tray thread's message pump. macOS cannot:
//! `NSStatusItem` is main-thread-only, Carbon's `RegisterEventHotKey` delivers
//! to the main run loop, and the AppKit call that hands the revealed window its
//! keyboard focus has to run on the main thread too. So everything here is
//! polled from `App::logic`, which eframe calls on every tick *including* the
//! ticks where the window is hidden and `App::ui` is skipped
//! (`native/epi_integration.rs`: `app.logic(..)` is unconditional, `app.ui(..)`
//! sits behind `if is_visible`). Putting any of it in `ui()` is exactly why the
//! port shipped a popup that appeared once and never again — and it is why the
//! status item would be dead in the one state it exists for.
//!
//! # Why the OS objects are built in eframe's creator closure
//!
//! [`start`] runs before `eframe::run_native` and only records what to build;
//! [`start_on_event_loop`] does the building, from the creator closure, where
//! `NSApplication` exists and we are provably on the main thread. Both the
//! status item and the hotkey manager need that.
//!
//! # Why the event handlers wake the context instead of feeding a channel
//!
//! `muda` offers a receiver *or* a global handler, not both, and the status
//! item's click action ([`status_item`]) and the Carbon hotkey handler
//! ([`hotkey_carbon`]) are callbacks with no receiver at all. A receiver is
//! only readable from a tick, and a tick
//! only happens when something wakes the event loop — which a status-item click
//! or a Carbon hotkey does not do on its own: winit services the AppKit event,
//! eframe sees no pending repaint and goes straight back to sleep. Polling a
//! receiver would therefore make a tray click land whenever the *timer* next
//! woke us, up to a full interval later.
//!
//! So the handlers do what `tray_bridge`'s do on Windows: raise a flag and wake
//! the app (`request_repaint` here, `force_foreground` there). Every decision
//! and every side effect still happens in [`tick`] — the handlers hold no
//! policy, and nothing is handled from `ui()`.
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
//! The status item makes that wake schedule denser — see [`next_wake`] — but
//! only by what the petal countdown actually needs to be drawn.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use eframe::egui;
use muda::{Menu, MenuEvent, MenuId, MenuItem};
use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{ClassType, sel};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameVibrantDark, NSApplication,
    NSApplicationActivationOptions, NSEvent, NSRunningApplication, NSScreen, NSView,
    NSVisualEffectBlendingMode, NSVisualEffectMaterial, NSVisualEffectState, NSVisualEffectView,
    NSWindowCollectionBehavior, NSWindowOrderingMode, NSWorkspace,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use crate::config;
use crate::config_watcher::{self, ConfigChange};
use crate::daisy;
use crate::hotkey::{self, Hotkey};
use crate::nudge_state;
use crate::screen_pick;
use crate::sprite::ScreenGeometry;

pub mod front_window;
pub mod hotkey_carbon;
pub mod key_monitor;
pub mod keycode;
pub mod settings_lock;
pub mod status_item;
pub mod symbolic;
pub use settings_lock::acquire_settings_singleton;
use status_item::StatusItem;

// ---------- shared state ---------------------------------------------------

/// The armed interval: when the next nudge is due and how long the interval
/// was. `None` when no timer is armed — before the first close (spec §4 freezes
/// the timer until then) and between an expiry and the reset that follows it.
///
/// The duration is what the petal countdown needs; the deadline is what the
/// wake schedule needs.
#[derive(Clone, Copy)]
struct ArmedTimer {
    deadline: Instant,
    duration: Duration,
}

static TIMER: Mutex<Option<ArmedTimer>> = Mutex::new(None);

/// Edge flags polled by `NudgeApp::ui`, mirroring the Windows ones so the
/// state transition on expiry is the app's existing code path, not a second
/// implementation of it.
static TRAY_CLICKED: AtomicBool = AtomicBool::new(false);
static TIMER_FIRED: AtomicBool = AtomicBool::new(false);

/// Set by the `config_watcher` callback when `config.json` changes on disk,
/// drained by `tick`.
static CONFIG_RELOAD_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Raised by the status-item left click, by the menu's "Show Nudge" item and by
/// the global hotkey — the three manual ways to summon the popup. Drained by
/// `tick`, which turns it into the same `TRAY_CLICKED` edge Windows raises.
static MANUAL_OPEN_REQUESTED: AtomicBool = AtomicBool::new(false);

/// Raised by the menu's "Settings" / "Quit" items, drained by `tick`. The
/// handler deliberately performs neither action itself: spawning a process and
/// calling `exit` from inside an AppKit menu callback would run them with the
/// menu's tracking loop still on the stack.
static SETTINGS_REQUESTED: AtomicBool = AtomicBool::new(false);
static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

/// `true` while the popup is on screen. `tick` parks on it exactly as the
/// tray thread does: while the user is looking at the popup we neither count
/// down nor animate. Starts `true` because the app opens with the launch popup.
///
/// Raised by [`reveal`], the moment the window is ordered in, and by
/// `App::show_popup` a frame or more later when `ui()` catches up; cleared only
/// by `App::hide_popup`. Both raise sites matter — see [`reveal`] for what the
/// gap between them used to cost.
static POPUP_VISIBLE: AtomicBool = AtomicBool::new(true);

/// The application that had the keyboard when the popup took it, by pid:
/// what [`hand_back_activation`] gives it back to (spec §1). Written by
/// [`focus_popup`] on the first activation of a show, taken by the hide.
/// `None` while hidden, and while shown when nothing else was active at
/// the moment the popup took the keyboard.
static PREVIOUS_FRONT: Mutex<Option<i32>> = Mutex::new(None);

/// The display spec §1 chose the last time the popup was shown, by its
/// frame in AppKit's screen space — the one identity a display keeps across
/// `NSScreen::screens` calls, whose order is not promised. Written by
/// [`choose_screen`], read by [`screen_geometry`]; `None` until the first
/// choice, when the window's own screen stands in.
static CHOSEN_SCREEN: Mutex<Option<NSRect>> = Mutex::new(None);

/// What [`start`] recorded for [`start_on_event_loop`] to build. Taken once.
static PENDING_START: Mutex<Option<(Option<Hotkey>, PathBuf, config::Config)>> = Mutex::new(None);

/// A sprite change from a config reload, waiting for the app to apply it
/// while the popup is hidden (spec §10). The newest change replaces an
/// older one still waiting.
static PENDING_SPRITE: Mutex<Option<config::SpriteConfig>> = Mutex::new(None);

/// A clone of the egui context, so an AppKit / Carbon callback on the main
/// thread can wake the event loop. `egui::Context` is `Send + Sync` and
/// `request_repaint` is the documented way to do this from anywhere.
static WAKE_CTX: Mutex<Option<egui::Context>> = Mutex::new(None);

thread_local! {
    /// The status item, the hotkey manager and their bookkeeping.
    ///
    /// Thread-local because [`StatusItem`] is `!Send` (it wraps AppKit
    /// objects behind `Retained`), and it must be *this* thread's — the main
    /// one — for its `NSStatusItem` to exist at all. Everything that touches
    /// it ([`start_on_event_loop`], [`tick`]) is called by eframe on the main
    /// thread.
    static UI: RefCell<Option<UiState>> = const { RefCell::new(None) };
}

struct UiState {
    /// `None` when the status item could not be built. It is the only part of
    /// this state that is optional-because-it-failed, and nothing else depends
    /// on it: the hotkey and the live reload below are what keep working when
    /// the menu bar refuses us.
    tray: Option<StatusItem>,
    /// The status item retains the `NSMenu`, not the muda `Menu` that routes
    /// its clicks to `MenuEvent`; and the menu keeps only weak references to
    /// its items. So we own both.
    _menu: Menu,
    _menu_items: [MenuItem; 3],
    /// Dedupe keys, so `set_frame` / `set_tooltip` are only called when what
    /// they would display actually changes. The icon key is
    /// `(petals attached, drifting petal + its progress in percent)`.
    last_icon_key: Option<(u8, Option<(u8, u16)>)>,
    last_tooltip_minutes: Option<u64>,
    reload: ReloadState,
}

/// Everything a `config.json` change needs, kept apart from the status item on
/// purpose.
///
/// These four fields used to sit in `UiState` next to a non-optional
/// status item, which made them hostage to it: a status item that failed to
/// build took the whole struct with it, and `apply_config_reload` — which
/// early-returns on an absent `UiState` — then dropped every later edit to
/// `config.json` for the lifetime of the process. Nothing here touches AppKit
/// unless a hotkey is actually being re-registered, so this is also the part
/// of the module that can be exercised without a menu bar.
struct ReloadState {
    /// The binding currently registered. Unregistering is what dropping it
    /// does, so a config reload is `take()` followed by a fresh [`register`].
    registered: Option<hotkey_carbon::Registration>,
    /// Where the config lives, and what is currently in effect — the baseline
    /// a reload diffs against, exactly as the tray thread keeps it on Windows.
    config_path: PathBuf,
    previous_cfg: config::Config,
}

// ---------- lifecycle ------------------------------------------------------

/// Record what the OS integration should be, to be built once the event loop
/// owns the main thread. Called from `main` before `eframe::run_native`.
pub fn start(hotkey: Option<Hotkey>, config_path: PathBuf, initial: config::Config) {
    *PENDING_START.lock().unwrap() = Some((hotkey, config_path, initial));
}

/// Build the OS integration. Called from eframe's creator closure, i.e. on the
/// main thread with a live `NSApplication` — the two preconditions the status
/// item and the Carbon hotkey both have.
pub fn start_on_event_loop(ctx: &egui::Context) {
    *WAKE_CTX.lock().unwrap() = Some(ctx.clone());

    // The launch popup is a show like any other (spec §1, "on every trigger
    // alike"), and this runs before its first frame — so before
    // `show_popup` makes Nudge the frontmost application.
    if let Some(mtm) = MainThreadMarker::new() {
        choose_screen(mtm);
    }

    let Some((hotkey, config_path, initial)) = PENDING_START.lock().unwrap().take() else {
        // No `start` — a test harness driving `NudgeApp` directly. The timer
        // half of this module works without any of the below.
        return;
    };

    let (menu, menu_items) = build_menu();
    install_event_handlers(menu_items.each_ref().map(|item| item.id().clone()));

    // Left click summons the popup, right click drops `menu` — the status
    // item's own action routes the two (`status_item`), matching the Windows
    // tray exactly.
    let tray = match MainThreadMarker::new()
        .ok_or_else(|| "not on the main thread".to_string())
        .and_then(|mtm| StatusItem::new(mtm, &menu))
    {
        Ok(t) => Some(t),
        Err(e) => {
            // Non-fatal, deliberately: without a status item the app is still
            // a working nudger (timer + hotkey + live config reload), just
            // without its menu-bar entry point. Refusing to start would be a
            // worse trade.
            //
            // Falling through — rather than returning — is the whole point.
            // An early return here would skip the hotkey manager below and
            // leave `UI` as `None`, which silently disables
            // `apply_config_reload` for the rest of the process: every later
            // edit to config.json would be drained by `tick` and dropped.
            eprintln!("[nudge] menu-bar status item unavailable: {e}");
            None
        }
    };

    let registered = hotkey.as_ref().and_then(try_register_hotkey);

    UI.with_borrow_mut(|slot| {
        *slot = Some(UiState {
            tray,
            _menu: menu,
            _menu_items: menu_items,
            last_icon_key: None,
            last_tooltip_minutes: None,
            reload: ReloadState {
                registered,
                config_path,
                previous_cfg: initial,
            },
        });
    });
}

fn build_menu() -> (Menu, [MenuItem; 3]) {
    let menu = Menu::new();
    let items = nudge_state::TRAY_MENU_LABELS.map(|label| MenuItem::new(label, true, None));
    for item in &items {
        // A menu we just created cannot reject an append; if it somehow did,
        // the user would see a short menu rather than a crashed app.
        if let Err(e) = menu.append(item) {
            eprintln!("[nudge] could not append tray menu item: {e}");
        }
    }
    (menu, items)
}

/// Wire the menu's event source. Each handler raises a flag and wakes the
/// event loop; `tick` decides what any of it means (see the module docs).
/// The status item's left click goes the same way, straight from its action
/// in [`status_item`].
fn install_event_handlers([show_id, settings_id, quit_id]: [MenuId; 3]) {
    MenuEvent::set_event_handler(Some(move |event: MenuEvent| {
        if event.id == show_id {
            request_manual_open();
        } else if event.id == settings_id {
            SETTINGS_REQUESTED.store(true, Ordering::SeqCst);
            wake();
        } else if event.id == quit_id {
            QUIT_REQUESTED.store(true, Ordering::SeqCst);
            wake();
        }
    }));

    // The hotkey's handler is installed by `hotkey_carbon::register`, on the
    // Carbon application event target and for the press edge only — there is
    // no release edge to filter, because none is subscribed.
}

fn request_manual_open() {
    MANUAL_OPEN_REQUESTED.store(true, Ordering::SeqCst);
    wake();
}

/// Wake the event loop so `logic` → [`tick`] runs now rather than at the next
/// scheduled repaint. The macOS counterpart of `tray_bridge::wake_eframe`.
fn wake() {
    if let Some(ctx) = WAKE_CTX.lock().unwrap().as_ref() {
        ctx.request_repaint();
    }
}

// ---------- the seam -------------------------------------------------------

/// No-op: nothing on this side needs the window handle. The reveal path goes
/// through `NSApplication` and egui's viewport commands, both of which
/// address the app's own main window without one.
///
/// Unreachable in the macOS *binary* — `app.rs` only captures a handle under
/// `cfg(windows)` — but the facade is a set, and the alternative to keeping the
/// odd one out is a cfg in the caller.
//
// `allow`, not `expect`: in the library half of the same build this is
// reachable public API, so an `expect` would be unfulfilled there.
#[allow(dead_code)]
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

/// Arm the timer. `duration` is what the petal countdown draws against;
/// `deadline` is what the wake schedule and the tooltip count down to.
///
/// Wakes the event loop, because nothing else here will. A new deadline is
/// only ever set from `hide_popup`, and `tick` refuses to schedule anything
/// while `POPUP_VISIBLE` is still true — which it is at this point, since
/// `hide_popup` clears it one line later. So the frame that turns this
/// deadline into a `request_repaint_after` has to be asked for; without the
/// wake the only thing producing it is the popup's own 1 Hz repaint request,
/// issued earlier in the same `ui()` pass for an unrelated reason.
pub fn set_timer_state(deadline: Instant, duration: Duration) {
    // Scoped so the lock is released before `wake` — it takes a different
    // mutex and calls into egui, and nothing here needs both at once.
    {
        *TIMER.lock().unwrap() = Some(ArmedTimer { deadline, duration });
    }
    wake();
}

pub fn set_popup_visible(visible: bool) {
    POPUP_VISIBLE.store(visible, Ordering::SeqCst);
    if !visible {
        // The blur panel hangs off the *window*, which is ordered out rather
        // than destroyed, so nothing else takes it down — left alone it keeps
        // frosting its patch of desktop for the whole interval the popup is
        // away. `sync_card_backdrop` unhides it on the next open.
        hide_card_backdrop();
        hand_back_activation();
    }
}

/// Spec §1: the popup "hands [the keyboard] back to the application that
/// had it when it closes". AppKit does not do that on its own: ordering the
/// window out leaves this app the active one with no key window, so every
/// keystroke goes nowhere until the user clicks, and the next show reads
/// nudge as the frontmost application, finds no ordinary-level window of
/// its own and falls through to the pointer's display.
///
/// Only while this app still is the active one: a Leave by focus loss means
/// the user has already put another application in front, and taking that
/// from them is the opposite of handing back. The remembered application
/// gets the activation; one that is gone, or none remembered, means hiding
/// instead, which has the system pick the next application the way ⌘H does.
fn hand_back_activation() {
    let previous = PREVIOUS_FRONT.lock().unwrap().take();
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(mtm);
    // SAFETY: plain accessors and requests on the shared application, on
    // the main thread; the running application is returned retained.
    unsafe {
        if !app.isActive() {
            return;
        }
        let target = previous
            .and_then(|pid| NSRunningApplication::runningApplicationWithProcessIdentifier(pid))
            .filter(|target| !target.isTerminated());
        let handed = target.is_some_and(|target| {
            let options = NSApplicationActivationOptions::empty();
            // macOS 14 made activation cooperative: the active application
            // names the one it hands over to. Older systems take the plain
            // request, and do not have the selector.
            if target.respondsToSelector(sel!(activateFromApplication:options:)) {
                target.activateFromApplication_options(
                    &NSRunningApplication::currentApplication(),
                    options,
                )
            } else {
                target.activateWithOptions(options)
            }
        });
        if !handed {
            app.hide(None);
        }
    }
}

/// Note which application is about to lose the keyboard to the popup, for
/// [`hand_back_activation`] — once per show: the first call of a show is
/// the one made before the activation, and the retries that follow it on a
/// popup not yet key would otherwise read this app as the previous one.
fn remember_front_application() {
    let mut previous = PREVIOUS_FRONT.lock().unwrap();
    if previous.is_some() {
        return;
    }
    // SAFETY: `sharedWorkspace` is a plain class accessor; the frontmost
    // application is returned retained, and `processIdentifier` only reads it.
    let pid = unsafe {
        NSWorkspace::sharedWorkspace()
            .frontmostApplication()
            .map(|front| front.processIdentifier())
    };
    if pid.is_some_and(|pid| pid != std::process::id() as i32) {
        *previous = pid;
    }
}

/// Signal that `config.json` changed on disk. Idempotent — N saves before
/// the next `tick` collapse into one re-read, which is the correct behaviour.
pub fn request_config_reload() {
    CONFIG_RELOAD_REQUESTED.store(true, Ordering::SeqCst);
    // The hotkey is live-reloadable, so a reload must not sit unapplied until
    // the timer happens to wake us.
    wake();
}

/// The sprite change parked by the last reload, if any; `NudgeApp` drains it.
pub fn take_pending_sprite() -> Option<config::SpriteConfig> {
    PENDING_SPRITE.lock().unwrap().take()
}

/// One pass of the OS integration, called from `NudgeApp::logic` on every tick.
///
/// Drains everything the event handlers raised, then either re-arms the wake
/// for the remaining time — refreshing the status item on the way — or, once
/// the deadline has passed, raises `TIMER_FIRED` and claims a reveal. It
/// deliberately does *not* perform the state transition itself: raising the
/// flag routes expiry into the same `ui()` branch Windows uses, so there is one
/// definition of what a timer-fired open does.
///
/// Returns whether it claimed a reveal ([`reveal`]); the caller then places
/// the window and orders it in.
pub fn tick(ctx: &egui::Context) -> bool {
    if QUIT_REQUESTED.swap(false, Ordering::SeqCst) {
        std::process::exit(0);
    }
    if SETTINGS_REQUESTED.swap(false, Ordering::SeqCst) {
        spawn_settings_process();
    }
    reap_settings_children();
    if CONFIG_RELOAD_REQUESTED.swap(false, Ordering::SeqCst) {
        apply_config_reload();
    }

    let popup_visible = POPUP_VISIBLE.load(Ordering::SeqCst);

    if MANUAL_OPEN_REQUESTED.swap(false, Ordering::SeqCst) && !popup_visible {
        // Same edge the Windows tray raises; `ui()` turns it into a
        // `TriggerSource::Manual` open.
        set_tray_clicked();
        reveal(ctx);
        return true;
    }

    if popup_visible {
        // The popup is up: the countdown is paused and `ui()` is driving its
        // own 1 Hz repaint, so there is nothing to schedule here.
        return false;
    }

    let mut slot = TIMER.lock().unwrap();
    let Some(timer) = *slot else {
        return false;
    };

    // Spec §5's timeline lives in `daisy::frame_at` (pure + tested); this loop
    // keeps only the wall-clock measurement and the AppKit I/O — the same
    // split the Windows tray thread makes. `elapsed` is allowed to grow past
    // `duration` so the last petal's fall can finish before the popup takes
    // focus.
    let now = Instant::now();
    let elapsed = if now >= timer.deadline {
        timer.duration + now.duration_since(timer.deadline)
    } else {
        timer
            .duration
            .saturating_sub(timer.deadline.duration_since(now))
    };
    let frame = daisy::frame_at(timer.duration, elapsed);
    let remaining = timer.deadline.saturating_duration_since(now);

    refresh_status_item(&frame, remaining);

    if !frame.should_fire {
        drop(slot);
        ctx.request_repaint_after(next_wake(&frame, remaining));
        return false;
    }

    // Expired. Disarm so one deadline fires once: every close of a
    // timer-fired popup resets the timer (`nudge_state::decide_close`), and
    // that reset is what re-arms us through `set_timer_state`.
    *slot = None;
    drop(slot);

    TIMER_FIRED.store(true, Ordering::SeqCst);
    reveal(ctx);
    true
}

/// Claim the popup for a reveal. Shared by the timer expiry and every manual
/// trigger, so all four entry points have identical side effects. Ordering
/// the window in is `NudgeApp::logic`'s, after it has placed the window.
///
/// The `request_repaint` is not redundant: the window is hidden right now,
/// so `ui()` is being skipped and cannot schedule the frame that will consume
/// the flag we just raised.
///
/// # Why this claims `POPUP_VISIBLE` itself
///
/// `POPUP_VISIBLE` used to be written only by `show_popup`, i.e. from `ui()`,
/// which cannot run in the same frame as this call: eframe samples
/// `is_visible` at the top of the frame, before `logic()`, and gates `app.ui`
/// on that sample (`glow_integration.rs` → `epi_integration.rs`). So between a
/// reveal and its `show_popup` there is always at least one further `tick`
/// that still saw the popup as hidden — and that tick would happily disarm the
/// timer and raise `TIMER_FIRED` into a card `ui()` is about to open manually,
/// where the flag is dropped. The deadline is then gone with nothing left to
/// re-arm it (a manual Esc yields `reset_timer: None`) and the app never
/// nudges again.
///
/// Claiming visibility in the frame the window is ordered in — which is what
/// actually happens — closes that window: from here until `hide_popup`,
/// `tick` parks, so the expiry cannot be consumed by anything other than the
/// card `ui()` is about to show. An expiry that fell inside a manual reveal is
/// simply still armed and still overdue when that card closes, and fires then.
fn reveal(ctx: &egui::Context) {
    // First, while the user's application is still the frontmost one: the
    // caller's `focus_popup` activation makes it Nudge, whose only window is
    // the floating popup — not an ordinary-level window, so the read would
    // find nothing and the rule would fall through to the pointer.
    if let Some(mtm) = MainThreadMarker::new() {
        choose_screen(mtm);
    }
    set_popup_visible(true);
    ctx.request_repaint();
}

/// Spec §1, "Which monitor": the display holding the frontmost
/// application's front window, else the one under the pointer, else the
/// primary — decided here, at the moment the popup is shown, and remembered
/// in [`CHOSEN_SCREEN`] for `screen_geometry` to lay the window out on.
///
/// The arithmetic is `screen_pick::pick_screen`, over rects in one space:
/// AppKit points with a top-left origin. The window list reports bounds in
/// CoreGraphics' global space, which is already that; screens and the
/// pointer come from AppKit with a bottom-left origin and are flipped
/// through the primary screen's height, as `screen_geometry` flips them.
fn choose_screen(mtm: MainThreadMarker) {
    let screens = NSScreen::screens(mtm);
    // SAFETY: the array is a fresh retained copy; `firstObject` only reads it.
    let Some(primary) = (unsafe { screens.firstObject() }) else {
        return;
    };
    let primary_height = primary.frame().size.height;
    let flip = |r: NSRect| {
        let top = primary_height - (r.origin.y + r.size.height);
        egui::Rect::from_min_size(
            egui::pos2(r.origin.x as f32, top as f32),
            egui::vec2(r.size.width as f32, r.size.height as f32),
        )
    };
    let frames: Vec<NSRect> = screens.iter().map(|s| s.frame()).collect();
    let rects: Vec<egui::Rect> = frames.iter().copied().map(flip).collect();
    let front = front_window::front_window_bounds().map(|r| {
        egui::Rect::from_min_size(
            egui::pos2(r.origin.x as f32, r.origin.y as f32),
            egui::vec2(r.size.width as f32, r.size.height as f32),
        )
    });
    // SAFETY: a plain class accessor with no preconditions.
    let mouse = unsafe { NSEvent::mouseLocation() };
    let pointer = egui::pos2(mouse.x as f32, (primary_height - mouse.y) as f32);
    let chosen = screen_pick::pick_screen(front, pointer, &rects);
    *CHOSEN_SCREEN.lock().unwrap() = frames.get(chosen).copied();
}

/// Reveal-and-focus, called on every path that puts the popup back on screen.
///
/// `ViewportCommand::Focus` alone is not enough: the window appears, but
/// `viewport().focused` stays false and every keystroke is swallowed — a
/// popup you cannot dismiss with Esc. A background app has to ask AppKit for
/// activation first; the viewport command then lands on an app the window
/// server is willing to give the keyboard to. With
/// `ActivationPolicy::Accessory` (set in `main`) this activation is the *only*
/// thing that makes the app able to take the keyboard at all.
///
/// `NSApplication` is main-thread-only, and the marker is the type system
/// saying so. In the app it is always present — eframe calls `logic()` and
/// `ui()` from the event loop on the main thread. The `None` arm is for
/// `tests/kittest_card.rs`, which drives a real `NudgeApp` from a libtest
/// worker thread against no window server at all: there is nothing to
/// activate there, and asking anyway is the unsound path, not the safe one.
pub fn focus_popup(ctx: &egui::Context) {
    if let Some(mtm) = MainThreadMarker::new() {
        remember_front_application();
        let app = NSApplication::sharedApplication(mtm);
        // SAFETY: plain accessors on the shared application, on the main
        // thread. A hide after the previous close (`hand_back_activation`)
        // keeps the window off screen until the app is unhidden.
        unsafe {
            if app.isHidden() {
                app.unhideWithoutActivation();
            }
        }
        #[allow(deprecated)]
        app.activateIgnoringOtherApps(true);
    }
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
}

// ---------- window placement -----------------------------------------------

/// The monitor the spotlight window goes on, in egui points with a top-left
/// origin: its whole frame and the part the menu bar and the Dock leave
/// free. `NudgeApp::place_window` makes the window the latter (spec §1).
///
/// Which monitor is the one [`choose_screen`] picked when the popup was
/// shown — looked up afresh among today's screens on every call, so its
/// work area is re-read when the Dock moves and a display change under a
/// visible popup lays it out again (§10). The window's own screen stands in
/// when nothing has been chosen yet, and when the chosen display is gone:
/// AppKit has then moved the window somewhere, and that is where it is.
///
/// This cannot be known in the `ViewportBuilder` the way the position is on
/// Windows, because no screen is knowable before the event loop exists — and
/// the Win32 answer is the wrong shape anyway: `GetSystemMetrics` returns
/// physical pixels while egui wants points, so a literal port would double
/// the offset on a Retina display. AppKit's points carry the backing scale
/// already; only the zoom factor separates them from egui's.
///
/// `None` off the main thread (kittest) and before the window has a handle.
pub fn screen_geometry(ctx: &egui::Context, frame: &eframe::Frame) -> Option<ScreenGeometry> {
    let mtm = MainThreadMarker::new()?;
    let Ok(handle) = frame.window_handle() else {
        return None;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return None;
    };
    // SAFETY: the handle is winit's own content view, alive for as long as the
    // window is, and we are on the thread that owns it.
    let content: &NSView = unsafe { &*appkit.ns_view.as_ptr().cast::<NSView>() };
    let screens = NSScreen::screens(mtm);
    let chosen = *CHOSEN_SCREEN.lock().unwrap();
    // A window off every screen has none; the main one is where it will show.
    let screen = chosen
        .and_then(|frame| screens.iter().find(|s| s.frame() == frame))
        .map(|s| s.retain())
        .or_else(|| content.window().and_then(|w| w.screen()))
        .or_else(|| NSScreen::mainScreen(mtm))?;
    // AppKit's screen space has its origin at the primary screen's bottom
    // left, y up. winit flips through the primary screen's height too, so
    // what goes back out through `ViewportCommand::OuterPosition` lands where
    // this read it.
    // SAFETY: the array is a fresh retained copy; `firstObject` only reads it.
    let primary = unsafe { screens.firstObject() }?;
    let primary_height = primary.frame().size.height;
    let zoom = f64::from(ctx.zoom_factor());
    let flip = |r: NSRect| {
        let top = primary_height - (r.origin.y + r.size.height);
        egui::Rect::from_min_size(
            egui::pos2((r.origin.x / zoom) as f32, (top / zoom) as f32),
            egui::vec2((r.size.width / zoom) as f32, (r.size.height / zoom) as f32),
        )
    };
    Some(ScreenGeometry {
        monitor: flip(screen.frame()),
        work: flip(screen.visibleFrame()),
    })
}

// ---------- status item ----------------------------------------------------

/// Redraw the petal countdown and re-word the tooltip, both only when what
/// they would show has changed — spec §5's "updated once per minute" for the
/// tooltip, and for the icon the whole cost of a status-item repaint, which
/// runs from one daisy render per display density through to a menu-bar
/// composite.
fn refresh_status_item(frame: &daisy::DaisyFrame, remaining: Duration) {
    let minutes = nudge_state::displayed_minutes(remaining);
    let drift_key = frame
        .drift
        .map(|d| (d.petal_index, (d.progress * 100.0) as u16));
    let icon_key = (frame.petals_remaining, drift_key);

    UI.with_borrow_mut(|slot| {
        let Some(ui) = slot.as_mut() else {
            return;
        };
        // Destructured so the tray and its two dedupe keys are disjoint
        // borrows rather than one whole-struct one.
        let UiState {
            tray,
            last_icon_key,
            last_tooltip_minutes,
            ..
        } = ui;
        // No status item (see `start_on_event_loop`): nothing to draw, and the
        // dedupe keys stay untouched so a tray that never existed cannot leave
        // stale ones behind.
        let Some(tray) = tray.as_ref() else {
            return;
        };

        if *last_icon_key != Some(icon_key) {
            tray.set_frame(frame.petals_remaining, frame.drift);
            *last_icon_key = Some(icon_key);
        }

        if *last_tooltip_minutes != Some(minutes) {
            tray.set_tooltip(&nudge_state::tooltip_for_remaining(remaining));
            *last_tooltip_minutes = Some(minutes);
        }
    });
}

/// One drift frame, ~30 Hz — the cadence the Windows tray animates at.
const DRIFT_FRAME: Duration = Duration::from_millis(33);

/// Sleep floor, so a zero-length answer cannot turn the wake schedule into a
/// spin.
const MIN_WAKE: Duration = Duration::from_millis(20);

/// When to wake next. Pure, so the wake schedule can be asserted without a
/// window server.
///
/// Same schedule as the Windows tray loop: animate while a petal is falling,
/// otherwise sleep until the next thing that changes what the user sees — a
/// petal dropping, the tooltip's minute count changing, or the deadline. So an
/// idle 10-minute interval wakes ~22 times plus twelve 250 ms bursts of
/// animation, rather than running a standing timer.
fn next_wake(frame: &daisy::DaisyFrame, remaining: Duration) -> Duration {
    if frame.drift.is_some() {
        return DRIFT_FRAME;
    }

    let to_petal = frame.petal_duration.saturating_sub(frame.time_since_drop);
    // `tooltip_for_remaining` shows `ceil(remaining / 60)`, so its wording
    // changes when `remaining` next crosses a whole minute.
    let minutes = nudge_state::displayed_minutes(remaining);
    let to_minute = remaining.saturating_sub(Duration::from_secs(minutes.saturating_sub(1) * 60));

    remaining.min(to_petal).min(to_minute).max(MIN_WAKE)
}

// ---------- global hotkey --------------------------------------------------

/// The combination this app currently holds machine-wide, as far as this
/// process knows.
///
/// Set by whichever process binds — in practice the main one; the settings
/// window never registers anything. Its one use is telling "we already have
/// this" apart from "another application has this" when the exclusive probe
/// refuses, and that case is real and *local*: measured, an exclusive probe
/// collides with this process's own **non-exclusive** binding, while the same
/// probe cannot see another process's non-exclusive binding at all. So this
/// answers a question only the process holding the binding can be asked, and
/// the settings window has nothing to put in it.
static NUDGE_HOTKEY: Mutex<Option<Hotkey>> = Mutex::new(None);

/// Tell this process which combination Nudge itself holds. `None` for "none".
pub fn note_own_hotkey(hk: Option<Hotkey>) {
    *NUDGE_HOTKEY.lock().unwrap() = hk;
}

/// Whether `hk` is the combination Nudge itself holds. Reads a static rather
/// than the `UI` cell on purpose: this is called from inside a conflict probe
/// that a config reload can trigger while `UI` is already mutably borrowed.
fn holds_this_hotkey(hk: &Hotkey) -> bool {
    NUDGE_HOTKEY.lock().unwrap().as_ref() == Some(hk)
}

/// Register `hk`, returning the live binding. Failure is logged and never
/// fatal — §5: the app runs *without* a hotkey rather than falling back to
/// some other combination the user did not ask for.
///
/// The conflict assessment runs **before** the binding exists, so the
/// exclusive probe inside it cannot collide with our own registration and
/// report us to ourselves.
fn try_register_hotkey(hk: &Hotkey) -> Option<hotkey_carbon::Registration> {
    let label = hotkey::format_native(hk);
    match hotkey_carbon::assess(hk) {
        hotkey_carbon::HotkeyStatus::Free => {}
        status => eprintln!("[nudge] hotkey \"{label}\": {status}"),
    }
    match hotkey_carbon::register(hk) {
        Ok(registration) => {
            note_own_hotkey(Some(hk.clone()));
            Some(registration)
        }
        Err(e) => {
            // Deliberately not a fallback binding: taking over some other
            // shortcut the user never asked for would hide which one failed.
            eprintln!(
                "[nudge] registering hotkey \"{label}\" failed: {e} — running with no hotkey"
            );
            note_own_hotkey(None);
            None
        }
    }
}

// ---------- config live-reload ---------------------------------------------

/// Settings processes we have spawned and not yet reaped. See
/// [`spawn_settings_process`].
static SETTINGS_CHILDREN: Mutex<Vec<std::process::Child>> = Mutex::new(Vec::new());

/// Spawn a second instance of the binary with `--settings`, exactly as the
/// Windows tray menu does. The two processes communicate ONLY through
/// `config.json` (and `~/Library/LaunchAgents` for autostart). Fire-and-forget
/// as far as *policy* goes: a failed spawn is logged and ignored, since
/// settings are editable by hand per spec §5. `--settings` is single-instance
/// in its own right (see [`acquire_settings_singleton`]), so repeat clicks
/// cannot stack windows.
///
/// It is not fire-and-forget as far as the *process table* goes. Dropping the
/// `Child` does not reap it: on Unix an exited child stays a zombie until its
/// parent waits, and this parent is a menu-bar app that runs for days. The
/// identical Windows line has no such consequence — there the handle is closed
/// on drop and the process object goes away — which is why it was inherited
/// unexamined. So we keep every `Child` and [`reap_settings_children`] waits on
/// the exited ones from `tick`.
fn spawn_settings_process() {
    match std::env::current_exe() {
        Ok(exe) => match std::process::Command::new(exe).arg("--settings").spawn() {
            Ok(child) => SETTINGS_CHILDREN.lock().unwrap().push(child),
            Err(e) => eprintln!("[nudge] could not open settings: {e}"),
        },
        Err(e) => eprintln!("[nudge] could not locate own executable: {e}"),
    }
}

/// Wait on every settings process that has already exited, dropping it from
/// the list. Non-blocking: a settings window still on screen stays in the list
/// and is looked at again on the next tick.
fn reap_settings_children() {
    let mut children = SETTINGS_CHILDREN.lock().unwrap();
    if children.is_empty() {
        return;
    }
    reap_exited(&mut children);
}

/// The reaping itself, over a plain vector so it can be tested without
/// spawning the real settings UI.
fn reap_exited(children: &mut Vec<std::process::Child>) {
    children.retain_mut(|child| match child.try_wait() {
        // Exited and now waited for: no zombie left, stop tracking it.
        Ok(Some(_)) => false,
        // Still running.
        Ok(None) => true,
        // We will never learn anything more about this one; holding the
        // `Child` forever would leak the slot as surely as the zombie did.
        Err(e) => {
            eprintln!("[nudge] could not wait on the settings process: {e}");
            false
        }
    });
}

/// Re-read `config.json` and apply any live-meaningful diff, mirroring
/// `tray_bridge::apply_config_reload`: only the hotkey has live effect on a
/// running app, the other two fields are recorded so the next diff is computed
/// against truth.
///
/// A malformed file is tolerated the way `load_or_default` does it: log, keep
/// the previous in-memory config. The watcher must not be the thing that kills
/// the app over a typo.
fn apply_config_reload() {
    UI.with_borrow_mut(|slot| {
        let Some(ui) = slot.as_mut() else {
            // Only reachable when `start` was never called — a test harness
            // driving `NudgeApp` directly, which has no config to reload. A
            // failed status item no longer lands here; see `start_on_event_loop`.
            return;
        };
        reload_config(&mut ui.reload);
    });
}

/// The reload itself, over the state it actually needs — deliberately not over
/// `UiState`, so it holds no opinion about whether a status item exists.
fn reload_config(state: &mut ReloadState) {
    let (new_cfg, err) = config::load_or_default(&state.config_path);
    if let Some(e) = err {
        eprintln!("[nudge] config reload: {e} — keeping previous in-memory config");
        return;
    }

    for change in config_watcher::diff(&state.previous_cfg, &new_cfg) {
        match change {
            ConfigChange::Hotkey { from, to } => {
                eprintln!("[nudge] config reload: hotkey \"{from}\" → \"{to}\"");
                // Dropping the old registration is what unregisters it, and it
                // happens before the new one is attempted: two live bindings on
                // one combination would leave the OS delivering to whichever
                // the OS chose.
                state.registered = None;
                note_own_hotkey(None);
                let (parsed, invalid) = new_cfg.resolved_hotkey();
                if invalid {
                    eprintln!(
                        "[nudge] config reload: hotkey \"{to}\" is unparseable, leaving unbound"
                    );
                } else {
                    state.registered = try_register_hotkey(&parsed);
                }
            }
            ConfigChange::Interval => eprintln!(
                "[nudge] config reload: default_interval_minutes {} → {} \
                 (no live effect; applies at next startup)",
                state.previous_cfg.default_interval_minutes, new_cfg.default_interval_minutes
            ),
            ConfigChange::Autostart => eprintln!(
                "[nudge] config reload: autostart {} → {} \
                 (settings process owns the LaunchAgent)",
                state.previous_cfg.autostart, new_cfg.autostart
            ),
            ConfigChange::Sprite { to } => {
                eprintln!("[nudge] config reload: sprite → {to:?} (applied while hidden)");
                *PENDING_SPRITE.lock().unwrap() = Some(to);
                wake();
            }
        }
    }

    state.previous_cfg = new_cfg;
}

// ---------------------------------------------------------------------------
// Frosted glass behind the card (spec §2)
// ---------------------------------------------------------------------------

thread_local! {
    /// The one backdrop panel, kept across frames. `Retained` is `!Send`, and
    /// AppKit views are main-thread-only anyway — the same reason `UI` above
    /// is thread-local.
    static CARD_BACKDROP: RefCell<Option<Retained<NSVisualEffectView>>> =
        const { RefCell::new(None) };
}

/// How the popup window behaves toward Spaces, Mission Control and the
/// desktop. A window above the normal level is implicitly `Transient` unless
/// told otherwise, and transient windows are what "click wallpaper to reveal
/// desktop" sweeps off the screen.
///
/// * `CanJoinAllSpaces` — the popup appears on whichever Space is current,
///   rather than switching the user back to the one it was created on.
/// * `FullScreenAuxiliary` — it may appear over another app's full-screen
///   window.
/// * `Stationary` — reveal-desktop and Mission Control leave it in place.
/// * `IgnoresCycle` — never a target of the ⌘` window cycle.
const POPUP_COLLECTION_BEHAVIOR: NSWindowCollectionBehavior =
    NSWindowCollectionBehavior::CanJoinAllSpaces
        .union(NSWindowCollectionBehavior::FullScreenAuxiliary)
        .union(NSWindowCollectionBehavior::Stationary)
        .union(NSWindowCollectionBehavior::IgnoresCycle);

/// Put a real blur under the card, and keep it there.
///
/// Spec §2 asks for frosted glass: what is behind the window shows through
/// *blurred*. egui cannot do that — it draws into a texture and has no access
/// to the pixels behind the window — so the blur has to come from AppKit, and
/// `NSVisualEffectView` with `BehindWindow` blending is the only thing that
/// samples the desktop behind a window.
///
/// The panel is a **sibling** of winit's content view, ordered below it, not a
/// child of it. That is not a preference: glutin binds the GL context to the
/// content view itself rather than to a subview, and an AppKit subview always
/// composites over its superview's own drawing — so a child would frost the
/// card's text instead of what is behind the window.
///
/// Every step degrades to "no blur" rather than panicking. The parent here is
/// the window's frame view, whose shape is AppKit's business and not a
/// documented contract, so a future layout that this code does not recognise
/// must leave a working popup behind.
pub fn sync_card_backdrop(ctx: &egui::Context, frame: &eframe::Frame, card: egui::Rect) {
    // Off the main thread there is no AppKit to talk to. `tests/kittest_card.rs`
    // drives the real `NudgeApp` from a libtest worker, and it must not crash.
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let Ok(handle) = frame.window_handle() else {
        return;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return;
    };
    // SAFETY: the handle is winit's own content view, alive for as long as the
    // window is, and we are on the thread that owns it.
    let content: &NSView = unsafe { &*appkit.ns_view.as_ptr().cast::<NSView>() };
    let Some(parent) = (unsafe { content.superview() }) else {
        return;
    };

    // The shadow is AppKit's here, and egui draws none (`CARD_SHADOW`). The
    // window's alpha mask is then the card and nothing else, so the system
    // shadow traces the card's own shape. Enabled explicitly rather than left
    // to the default, because the pairing is what makes it correct: with
    // egui's shadow also in that alpha the silhouette is bigger than the card
    // and AppKit outlines *it*, laying a hard rim beside the contour.
    if let Some(window) = content.window() {
        window.setHasShadow(true);
        if unsafe { window.collectionBehavior() } != POPUP_COLLECTION_BEHAVIOR {
            unsafe { window.setCollectionBehavior(POPUP_COLLECTION_BEHAVIOR) };
        }
    }

    // egui points → AppKit points. Only the zoom factor separates them:
    // AppKit points already carry the display's backing scale, so this is the
    // identity at the default zoom on Retina and non-Retina alike.
    //
    // The rect is built in content-view space — which winit makes flipped, so
    // it is top-left-origin exactly like egui's — and AppKit performs the flip
    // into the unflipped parent. Doing the arithmetic by hand instead is how
    // the card ends up mirrored about the window's horizontal centre.
    let zoom = f64::from(ctx.zoom_factor());
    let in_content = NSRect::new(
        NSPoint::new(f64::from(card.min.x) * zoom, f64::from(card.min.y) * zoom),
        NSSize::new(
            f64::from(card.width()) * zoom,
            f64::from(card.height()) * zoom,
        ),
    );
    let target = content.convertRect_toView(in_content, Some(&parent));

    CARD_BACKDROP.with_borrow_mut(|slot| {
        let view = slot.get_or_insert_with(|| unsafe {
            let view = NSVisualEffectView::new(mtm);
            view.setMaterial(NSVisualEffectMaterial::HUDWindow);
            view.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
            // Not the default `FollowsWindowActiveState`: this is a menu-bar
            // agent that is almost never the frontmost app, and following the
            // window state would render the inactive, washed-out material
            // nearly every time the popup appears.
            view.setState(NSVisualEffectState::Active);
            // The card is dark whatever the system theme is (`set_theme(Dark)`
            // in `NudgeApp::new`), so the material behind it has to be too —
            // otherwise a light-mode Mac frosts it pale under a dark card.
            view.setAppearance(
                NSAppearance::appearanceNamed(NSAppearanceNameVibrantDark).as_deref(),
            );
            view.setWantsLayer(true);
            if let Some(layer) = view.layer() {
                // Clip the material to the card. Circular, matching egui's
                // `CornerRadius` — a continuous curve would visibly diverge
                // from the contour drawn on top of it.
                layer.setCornerRadius(f64::from(crate::app::CARD_RADIUS));
                layer.setMasksToBounds(true);
            }
            parent.addSubview_positioned_relativeTo(
                &view,
                NSWindowOrderingMode::NSWindowBelow,
                Some(content),
            );
            view
        });

        view.setHidden(false);
        if view.frame() != target {
            unsafe { view.setFrame(target) };
        }
    });
}

/// Take the blur away with the popup. Called by `set_popup_visible(false)`,
/// which is the app's single "the popup is going away" edge.
fn hide_card_backdrop() {
    if MainThreadMarker::new().is_none() {
        return;
    }
    CARD_BACKDROP.with_borrow(|slot| {
        if let Some(view) = slot {
            view.setHidden(true);
        }
    });
}

// ---------- fonts ----------------------------------------------------------

/// Where the modifier symbols come from. Apple Symbols is on the sealed
/// system volume of every macOS, and it is the only stock face checked here
/// that carries all eight of `⌃ ⌥ ⇧ ⌘ ⎋ ⇥ ⌫ ↩` (SF Pro and SF Mono are each
/// missing `⇥` and `↩`; Menlo and Lucida Grande also have the full set, but
/// they are text faces and this is only ever a fallback).
const SYMBOL_FONT_PATH: &str = "/System/Library/Fonts/Apple Symbols.ttf";
const SYMBOL_FONT_NAME: &str = "apple-symbols";

/// Append Apple Symbols behind egui's default families, so a shortcut written
/// the Mac way renders as symbols instead of tofu boxes.
///
/// It has to be a *fallback* rather than a replacement: the face is symbols
/// only, so promoting it would strip the Latin and Cyrillic egui's bundled
/// faces provide. egui resolves a glyph by walking the family's list in
/// order, which makes "append" exactly the right operation.
///
/// A missing or unreadable file is logged and otherwise ignored. The failure
/// mode is a hotkey label rendered as boxes — bad, but not worth refusing to
/// open the window over, and the label is still readable as words in
/// `config.json`.
pub fn install_native_fonts(ctx: &egui::Context) {
    let bytes = match std::fs::read(SYMBOL_FONT_PATH) {
        Ok(b) => b,
        Err(e) => {
            eprintln!(
                "[nudge] no symbol font at {SYMBOL_FONT_PATH} ({e}); shortcut symbols will not render"
            );
            return;
        }
    };
    let mut fonts = egui::FontDefinitions::default();
    fonts.font_data.insert(
        SYMBOL_FONT_NAME.to_owned(),
        std::sync::Arc::new(egui::FontData::from_owned(bytes)),
    );
    for family in [egui::FontFamily::Proportional, egui::FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push(SYMBOL_FONT_NAME.to_owned());
    }
    ctx.set_fonts(fonts);
}

/// The settings window's key source: an app-local `NSEvent` monitor.
///
/// **Main thread, with an `NSApplication` up** — call it from the eframe
/// creator closure, the same precondition [`start_on_event_loop`] has. `None`
/// means the monitor could not be installed and the hotkey row is the typed
/// field alone; nothing else about the window changes.
///
/// It is installed in the *settings* process only. The process that registers
/// the hotkey never reads a key, and the process that reads keys never
/// registers one — which is why pressing the app's own hotkey while the
/// recorder is armed opens the popup instead of being recorded.
pub fn new_edge_source(ctx: &egui::Context) -> Option<Box<dyn crate::recorder::EdgeSource>> {
    key_monitor::KeyMonitor::install(ctx.clone())
        .map(|m| Box::new(m) as Box<dyn crate::recorder::EdgeSource>)
}

/// Who else holds `hk`, in one line, or `None` when nothing we can see does.
///
/// A warning, never a refusal: a taken combination saves and registers
/// perfectly well, it simply never fires, and that is a different thing from
/// the unregistrable one §5 blocks the save on. Both sources are read; the
/// line says which one answered.
pub fn hotkey_conflict(hk: &Hotkey) -> Option<String> {
    match hotkey_carbon::assess(hk) {
        // `HeldByNudge` cannot happen here — measured, an exclusive probe
        // cannot see another process's non-exclusive binding, and the settings
        // window binds nothing of its own. If it ever does appear, it is us,
        // and warning the user about ourselves would be noise.
        hotkey_carbon::HotkeyStatus::Free | hotkey_carbon::HotkeyStatus::HeldByNudge => None,
        status => Some(status.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // === hotkey registration =================================================
    //
    // The table this backend used to own — our token to the crate's key code —
    // is gone, and with it the tests that walked it. What replaces it lives in
    // `keycode.rs`, where one array is proved total against the grammar in both
    // directions, and in `hotkey_carbon.rs`. Nothing is asserted here about the
    // *outcome* of a registration: on macOS success says nothing about who else
    // holds the combination, which is the measurement this whole change rests
    // on, and a test that asserted otherwise would be re-asserting the bug.

    #[test]
    fn the_process_can_be_told_which_hotkey_is_its_own() {
        // The settings window is a separate process from the one that binds,
        // so "Nudge already has this" is knowledge each process is handed
        // rather than something either can observe.
        let hk = hotkey::parse("Alt+Cmd+J").unwrap();
        note_own_hotkey(Some(hk.clone()));
        assert!(holds_this_hotkey(&hk));
        assert!(!holds_this_hotkey(&hotkey::parse("Alt+Cmd+K").unwrap()));
        note_own_hotkey(None);
        assert!(!holds_this_hotkey(&hk));
    }

    // === the wake schedule ===================================================
    //
    // A 10-minute interval throughout: 12 petals, one every 50s, drift capped
    // at 250ms.

    const INTERVAL: Duration = Duration::from_secs(600);

    /// The frame `tick` would compute with `remaining` left on the clock.
    fn frame_with(remaining: Duration) -> daisy::DaisyFrame {
        daisy::frame_at(INTERVAL, INTERVAL.saturating_sub(remaining))
    }

    fn wake_with(remaining: Duration) -> Duration {
        next_wake(&frame_with(remaining), remaining)
    }

    #[test]
    fn wake_never_overshoots_the_deadline() {
        for secs in [1, 5, 59, 60, 61, 300, 599] {
            let remaining = Duration::from_secs(secs);
            assert!(
                wake_with(remaining) <= remaining,
                "wake must not sleep past the deadline (remaining {secs}s)"
            );
        }
    }

    #[test]
    fn wake_lands_on_the_next_tooltip_wording_change() {
        // 2m05s left: the tooltip flips "~3 min" → "~2 min" in 5s, the next
        // petal is further out, so the tooltip is what sets the alarm.
        assert_eq!(wake_with(Duration::from_secs(125)), Duration::from_secs(5));
    }

    #[test]
    fn wake_lands_on_the_next_petal_drop_when_that_comes_first() {
        // 3m01s left: the tooltip boundary is 61s away and the petal is 1s
        // away (419s elapsed is 49s into the 9th petal's 50s life).
        assert_eq!(wake_with(Duration::from_secs(181)), Duration::from_secs(1));
    }

    #[test]
    fn wake_animates_while_a_petal_is_falling() {
        // 100ms into a petal drop (the second, at 75 s of a 600 s interval
        // with 16 petals), inside the 250ms drift window.
        let remaining = Duration::from_millis(524_900);
        assert!(
            frame_with(remaining).drift.is_some(),
            "this is a drift frame"
        );
        assert_eq!(wake_with(remaining), DRIFT_FRAME);
    }

    #[test]
    fn wake_keeps_animating_past_the_deadline_until_the_last_petal_lands() {
        // `should_fire` waits for the final drift, so the deadline passing is
        // not the end of the animation — the popup is owed to the user 250ms
        // later, and the frames in between are the last petal falling.
        let frame = daisy::frame_at(INTERVAL, INTERVAL + Duration::from_millis(200));
        assert!(!frame.should_fire, "the final drift is still running");
        assert_eq!(next_wake(&frame, Duration::ZERO), DRIFT_FRAME);
    }

    #[test]
    fn wake_has_a_floor_so_it_cannot_ask_for_a_zero_length_sleep() {
        // Nothing left to count down to and no drift to animate: without the
        // floor this is a request to be woken immediately, forever.
        let frame = daisy::frame_at(INTERVAL, INTERVAL * 2);
        assert!(frame.drift.is_none(), "long past the last drift");
        assert_eq!(next_wake(&frame, Duration::ZERO), MIN_WAKE);
    }

    // === the handler → tick → app edge =======================================

    /// Every flag `tick` reads is a process global, so the tests that drive it
    /// have to take turns. Poison is ignored: a panicking test has already
    /// failed and must not take the rest of the file down with it.
    static TICK_GLOBALS: Mutex<()> = Mutex::new(());

    fn tick_globals() -> std::sync::MutexGuard<'static, ()> {
        TICK_GLOBALS.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Put the module back in the state a fresh process starts in, so the
    /// order libtest happens to pick cannot change what a test sees.
    fn reset_globals() {
        let _ = take_tray_clicked();
        let _ = take_timer_fired();
        let _ = MANUAL_OPEN_REQUESTED.swap(false, Ordering::SeqCst);
        *TIMER.lock().unwrap() = None;
        set_popup_visible(false);
    }

    /// The middle link of the manual-open path: a handler raises a flag from
    /// outside any frame, and the *next* tick has to turn it into the
    /// `TRAY_CLICKED` edge `ui()` consumes. If `tick` dropped it, a status-item
    /// click while the popup is hidden — the only state the status item exists
    /// for — would do nothing at all.
    #[test]
    fn tick_turns_a_handler_flag_into_the_edge_the_app_reads() {
        let _guard = tick_globals();
        reset_globals();
        let ctx = egui::Context::default();

        set_popup_visible(false);
        request_manual_open();
        tick(&ctx);
        assert!(
            take_tray_clicked(),
            "a manual trigger raised while the card is hidden must reach the app"
        );

        // And with the card already up there is nothing to open, so the
        // trigger is consumed rather than left to fire later.
        set_popup_visible(true);
        request_manual_open();
        tick(&ctx);
        assert!(
            !take_tray_clicked(),
            "a trigger arriving while the card is already up must not queue up behind it"
        );
    }

    /// A reveal is not instantaneous: `ui()` cannot run in the same frame that
    /// `tick` revealed in, so there is always at least one more `tick` before
    /// the card is acknowledged. If the timer is allowed to expire in that gap
    /// it disarms itself and raises `TIMER_FIRED` into a card `ui()` is about
    /// to open as `Manual` — where the flag is dropped. A manual Esc then
    /// yields `reset_timer: None`, so nothing re-arms, and the app never
    /// nudges again.
    #[test]
    fn an_expiry_inside_a_manual_reveal_is_not_swallowed() {
        let _guard = tick_globals();
        reset_globals();
        let ctx = egui::Context::default();

        // The status item is clicked at the exact moment the interval runs
        // out: hidden card, deadline already a second in the past (well past
        // the 250 ms final drift, so `should_fire` is true).
        let interval = Duration::from_secs(600);
        set_timer_state(Instant::now() - Duration::from_secs(1), interval);
        request_manual_open();

        tick(&ctx);
        assert!(take_tray_clicked(), "the manual open reaches the app");
        assert!(
            !take_timer_fired(),
            "the manual branch returns before the expiry is even looked at"
        );

        // The frames between that reveal and the `show_popup` that
        // acknowledges it. This is where the deadline used to be thrown away.
        tick(&ctx);
        tick(&ctx);
        assert!(
            !take_timer_fired(),
            "an expiry must not be raised while a reveal is still unacknowledged"
        );
        assert!(
            TIMER.lock().unwrap().is_some(),
            "and the deadline must still be armed — nothing else would re-arm it"
        );

        // The user dismisses the manual card. `decide_close(Manual, Dismiss)`
        // leaves the timer alone, so the nudge that came due during the manual
        // open is still owed, and is delivered now.
        set_popup_visible(false);
        tick(&ctx);
        assert!(
            take_timer_fired(),
            "the overdue nudge fires once the manual card is out of the way"
        );
        assert!(
            TIMER.lock().unwrap().is_none(),
            "and one deadline still fires exactly once"
        );
    }

    /// `reveal` is what makes the assertion above hold, on both paths into it.
    #[test]
    fn a_reveal_claims_the_popup_as_visible() {
        let _guard = tick_globals();
        reset_globals();
        let ctx = egui::Context::default();

        reveal(&ctx);
        assert!(
            POPUP_VISIBLE.load(Ordering::SeqCst),
            "the window has been ordered in; `tick` must park until `hide_popup` says otherwise"
        );
    }

    /// A new deadline has to produce the tick that schedules it. Nothing else
    /// here will: `set_timer_state` is only ever called from `hide_popup`,
    /// where `POPUP_VISIBLE` is still true, and `tick` schedules nothing in
    /// that state. Before this, the frame that armed the real deadline came
    /// from the popup's own 1 Hz repaint request, issued a few lines earlier
    /// in `ui()` for an unrelated reason — correct by accident.
    #[test]
    fn arming_a_deadline_wakes_the_event_loop() {
        let _guard = tick_globals();
        reset_globals();

        // `start_on_event_loop` with no `PENDING_START` does exactly one
        // thing: publish the context the wake goes through. That is the state
        // the app is in from its creator closure onwards.
        let ctx = egui::Context::default();
        start_on_event_loop(&ctx);

        // Run passes until the context settles: a fresh `Context` asks for a
        // repaint on its first frames on its own, and a test that cannot tell
        // that apart from our wake would pass with the wake removed.
        let mut settled = false;
        for _ in 0..8 {
            ctx.begin_pass(egui::RawInput::default());
            let _ = ctx.end_pass();
            if !ctx.has_requested_repaint() {
                settled = true;
                break;
            }
        }
        assert!(
            settled,
            "the harness must start from a context with nothing pending, \
             or this test cannot see the wake it exists for"
        );

        set_timer_state(Instant::now() + Duration::from_secs(600), INTERVAL);
        assert!(
            ctx.has_requested_repaint(),
            "arming a deadline must ask for the frame that will schedule it"
        );

        *WAKE_CTX.lock().unwrap() = None;
    }

    // === config live-reload ==================================================

    /// The state a failed status item leaves behind: no tray, and a reload
    /// that must still run to completion in it — the defect was that this code
    /// was unreachable, because a tray failure returned before `UI` was ever
    /// populated and `apply_config_reload` then early-returned forever,
    /// silently discarding every later edit to `config.json`.
    ///
    /// The edited hotkey is `Ctrl+F21` on purpose. It parses (§5 accepts the
    /// whole F row, because Windows binds it) and macOS has no key code for
    /// it, so the reload runs its full length and is refused *before* any OS
    /// call — this test never takes a real key away from the machine running
    /// it, and it pins the §5 rule that a combination we cannot bind leaves
    /// the app unbound rather than falling back to something else.
    #[test]
    fn a_reload_applies_without_a_status_item_and_refuses_an_unbindable_hotkey() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.json");

        let before = config::Config {
            hotkey: "Ctrl+Shift+Space".to_string(),
            default_interval_minutes: 10.0,
            autostart: false,
            ..config::Config::default()
        };
        let after = config::Config {
            hotkey: "Ctrl+F21".to_string(),
            default_interval_minutes: 25.0,
            autostart: true,
            sprite: config::SpriteConfig {
                enabled: true,
                path: "/tmp/sprite.png".to_string(),
                x: -41,
                y: -277,
                card_width: 679,
            },
        };
        config::save(&path, &after).expect("write the edited config");

        let mut state = ReloadState {
            registered: None,
            config_path: path.clone(),
            previous_cfg: before,
        };
        let _guard = tick_globals();
        reload_config(&mut state);

        assert_eq!(
            take_pending_sprite(),
            Some(after.sprite.clone()),
            "the sprite block is parked for the app to apply while hidden (§10)"
        );

        assert_eq!(
            state.previous_cfg, after,
            "the baseline the next diff is computed against must be what is on disk"
        );
        assert!(
            state.registered.is_none(),
            "a combination macOS cannot bind must leave the app with no hotkey, \
             not with a fallback the user never asked for"
        );
        assert!(
            !holds_this_hotkey(&hotkey::parse("Ctrl+Shift+Space").unwrap()),
            "the old binding is dropped before the new one is attempted"
        );
    }

    /// The watcher must not be the thing that kills the app over a typo: a
    /// file that no longer parses leaves the in-memory config alone.
    #[test]
    fn a_malformed_config_keeps_the_previous_baseline() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.json");
        std::fs::write(&path, "{ not json").expect("write a broken config");

        let before = config::Config {
            hotkey: "Ctrl+Shift+Space".to_string(),
            default_interval_minutes: 10.0,
            autostart: false,
            ..config::Config::default()
        };
        let mut state = ReloadState {
            registered: None,
            config_path: path,
            previous_cfg: before.clone(),
        };
        reload_config(&mut state);

        assert_eq!(state.previous_cfg, before);
        assert_eq!(
            take_pending_sprite(),
            None,
            "nothing changed, nothing parked"
        );
    }

    /// The app drains the parked sprite from `logic`, which only runs when
    /// the event loop does. A reload arriving while the popup is hidden must
    /// not sit unapplied until the interval timer happens to wake us.
    #[test]
    fn a_sprite_change_wakes_the_event_loop() {
        let _guard = tick_globals();
        reset_globals();
        let ctx = egui::Context::default();
        start_on_event_loop(&ctx);
        let mut settled = false;
        for _ in 0..8 {
            ctx.begin_pass(egui::RawInput::default());
            let _ = ctx.end_pass();
            if !ctx.has_requested_repaint() {
                settled = true;
                break;
            }
        }
        assert!(settled, "the context must start with nothing pending");

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.json");
        let before = config::Config::default();
        let after = config::Config {
            sprite: config::SpriteConfig {
                enabled: true,
                path: "/tmp/sprite.png".to_string(),
                ..config::SpriteConfig::default()
            },
            ..config::Config::default()
        };
        config::save(&path, &after).expect("write the edited config");
        let mut state = ReloadState {
            registered: None,
            config_path: path,
            previous_cfg: before,
        };
        reload_config(&mut state);

        assert!(
            ctx.has_requested_repaint(),
            "a parked sprite change must ask for the frame that applies it"
        );
        assert_eq!(take_pending_sprite(), Some(after.sprite));
        *WAKE_CTX.lock().unwrap() = None;
    }

    // === spawned settings processes ==========================================

    /// Rust's `Child` does not reap on drop and this process has no
    /// `SIGCHLD`/`SIG_IGN` handler, so a settings window the user closes stays
    /// in the process table as a zombie until its parent waits for it — and
    /// the parent here is a menu-bar app that runs for days.
    #[test]
    fn an_exited_settings_child_is_waited_for_and_a_live_one_is_kept() {
        let long = std::process::Command::new("/bin/sleep")
            .arg("30")
            .spawn()
            .expect("spawn a long-lived child");
        let long_pid = long.id();
        let short = std::process::Command::new("/bin/sh")
            .args(["-c", "exit 0"])
            .spawn()
            .expect("spawn a short-lived child");
        let short_pid = short.id();
        let mut children = vec![long, short];

        let deadline = Instant::now() + Duration::from_secs(5);
        while children.len() == 2 && Instant::now() < deadline {
            reap_exited(&mut children);
            std::thread::sleep(Duration::from_millis(10));
        }

        assert_eq!(
            children.len(),
            1,
            "the exited child was dropped from the list"
        );
        assert_eq!(
            children[0].id(),
            long_pid,
            "and it was the exited one, not the running one"
        );

        // The list shrinking only proves we stopped tracking it. What the bug
        // was about is the process table, so ask the process table: a pid that
        // was never waited for would still be there, in state `Z`.
        let state = std::process::Command::new("/bin/ps")
            .args(["-o", "state=", "-p", &short_pid.to_string()])
            .output()
            .expect("ps must run");
        let state = String::from_utf8_lossy(&state.stdout).trim().to_string();
        assert!(
            state.is_empty(),
            "pid {short_pid} is still in the process table as {state:?} — it was not reaped"
        );

        let _ = children[0].kill();
        let _ = children[0].wait();
    }
}
