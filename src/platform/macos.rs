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
//! `tray_icon` / `muda` / `global_hotkey` each offer a receiver *or* a global
//! handler, not both. The receiver is only readable from a tick, and a tick
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
use global_hotkey::hotkey::{Code, HotKey as GlobalHotKey, Modifiers as GlobalModifiers};
use global_hotkey::{GlobalHotKeyEvent, GlobalHotKeyManager, HotKeyState};
use objc2_app_kit::NSApplication;
use objc2_foundation::MainThreadMarker;
use tray_icon::menu::{Menu, MenuEvent, MenuId, MenuItem};
use tray_icon::{MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent};

use crate::config;
use crate::config_watcher::{self, ConfigChange};
use crate::daisy;
use crate::hotkey::{
    self, Hotkey, MOD_ALT as MY_MOD_ALT, MOD_CTRL as MY_MOD_CTRL, MOD_SHIFT as MY_MOD_SHIFT,
    MOD_WIN as MY_MOD_WIN,
};
use crate::nudge_state;

pub mod settings_lock;
pub use settings_lock::acquire_settings_singleton;

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
static POPUP_VISIBLE: AtomicBool = AtomicBool::new(true);

/// One-shot: the spotlight window has been moved to its spec §1 position.
/// See [`apply_window_position`] for why this cannot happen in the viewport
/// builder.
static POSITION_APPLIED: AtomicBool = AtomicBool::new(false);

/// What [`start`] recorded for [`start_on_event_loop`] to build. Taken once.
static PENDING_START: Mutex<Option<(Option<Hotkey>, PathBuf, config::Config)>> = Mutex::new(None);

/// A clone of the egui context, so an AppKit / Carbon callback on the main
/// thread can wake the event loop. `egui::Context` is `Send + Sync` and
/// `request_repaint` is the documented way to do this from anywhere.
static WAKE_CTX: Mutex<Option<egui::Context>> = Mutex::new(None);

thread_local! {
    /// The status item, the hotkey manager and their bookkeeping.
    ///
    /// Thread-local because `tray_icon::TrayIcon` is `!Send` (it wraps AppKit
    /// objects behind `Retained`), and it must be *this* thread's — the main
    /// one — for its `NSStatusItem` to exist at all. Everything that touches
    /// it ([`start_on_event_loop`], [`tick`]) is called by eframe on the main
    /// thread.
    static UI: RefCell<Option<UiState>> = const { RefCell::new(None) };
}

struct UiState {
    tray: TrayIcon,
    /// The menu keeps only weak references to its items, so we own them.
    _menu_items: [MenuItem; 3],
    /// `None` when `GlobalHotKeyManager::new()` failed — the app runs on
    /// without a hotkey rather than refusing to start.
    hotkeys: Option<GlobalHotKeyManager>,
    /// The binding currently registered, so a config reload can unregister it.
    registered: Option<GlobalHotKey>,
    /// Where the config lives, and what is currently in effect — the baseline
    /// a reload diffs against, exactly as the tray thread keeps it on Windows.
    config_path: PathBuf,
    previous_cfg: config::Config,
    /// Dedupe keys, so `set_icon` / `set_tooltip` are only called when what
    /// they would display actually changes. The icon key is
    /// `(petals attached, drifting petal + its progress in percent)`.
    last_icon_key: Option<(u8, Option<(u8, u16)>)>,
    last_tooltip_minutes: Option<u64>,
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

    let Some((hotkey, config_path, initial)) = PENDING_START.lock().unwrap().take() else {
        // No `start` — a test harness driving `NudgeApp` directly. The timer
        // half of this module works without any of the below.
        return;
    };

    let (menu, menu_items) = build_menu();
    install_event_handlers(menu_items.each_ref().map(|item| item.id().clone()));

    let initial_rgba = daisy::render(daisy::PETAL_COUNT, None);
    let tray = match tray_icon::Icon::from_rgba(initial_rgba, daisy::ICON_SIZE, daisy::ICON_SIZE)
        .map_err(|e| e.to_string())
        .and_then(|icon| {
            TrayIconBuilder::new()
                .with_menu(Box::new(menu))
                // Left click summons the popup (the `TrayIconEvent` handler
                // below); the menu is the right-click gesture. Matches the
                // Windows tray exactly.
                .with_menu_on_left_click(false)
                // Refreshed by `tick` as soon as a timer is armed. Until then
                // the launch popup is up, so nobody reads this placeholder.
                .with_tooltip("Nudge")
                .with_icon(icon)
                .build()
                .map_err(|e| e.to_string())
        }) {
        Ok(t) => t,
        Err(e) => {
            // Non-fatal, deliberately: without a status item the app is still
            // a working nudger (timer + hotkey), just without its manual
            // entry point. Refusing to start would be a worse trade.
            eprintln!("[nudge] menu-bar status item unavailable: {e}");
            return;
        }
    };

    let hotkeys = match GlobalHotKeyManager::new() {
        Ok(m) => Some(m),
        Err(e) => {
            eprintln!("[nudge] global hotkey unavailable: {e}");
            None
        }
    };
    let registered = hotkeys
        .as_ref()
        .zip(hotkey.as_ref())
        .and_then(|(manager, hk)| try_register_hotkey(manager, hk));

    UI.with_borrow_mut(|slot| {
        *slot = Some(UiState {
            tray,
            _menu_items: menu_items,
            hotkeys,
            registered,
            config_path,
            previous_cfg: initial,
            last_icon_key: None,
            last_tooltip_minutes: None,
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

/// Wire the three event sources. Each handler raises a flag and wakes the
/// event loop; `tick` decides what any of it means (see the module docs).
fn install_event_handlers([show_id, settings_id, quit_id]: [MenuId; 3]) {
    TrayIconEvent::set_event_handler(Some(|event| {
        if matches!(
            event,
            TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            }
        ) {
            request_manual_open();
        }
    }));

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

    GlobalHotKeyEvent::set_event_handler(Some(|event: GlobalHotKeyEvent| {
        // Press only. Carbon delivers both edges and the release would
        // immediately re-open a popup the user just dismissed with the same
        // chord.
        if event.state == HotKeyState::Pressed {
            request_manual_open();
        }
    }));
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
pub fn set_timer_state(deadline: Instant, duration: Duration) {
    *TIMER.lock().unwrap() = Some(ArmedTimer { deadline, duration });
}

pub fn set_popup_visible(visible: bool) {
    POPUP_VISIBLE.store(visible, Ordering::SeqCst);
}

/// Signal that `config.json` changed on disk. Idempotent — N saves before
/// the next `tick` collapse into one re-read, which is the correct behaviour.
pub fn request_config_reload() {
    CONFIG_RELOAD_REQUESTED.store(true, Ordering::SeqCst);
    // The hotkey is live-reloadable, so a reload must not sit unapplied until
    // the timer happens to wake us.
    wake();
}

/// One pass of the OS integration, called from `NudgeApp::logic` on every tick.
///
/// Drains everything the event handlers raised, then either re-arms the wake
/// for the remaining time — refreshing the status item on the way — or, once
/// the deadline has passed, raises `TIMER_FIRED` and reveals the window. It
/// deliberately does *not* perform the state transition itself: raising the
/// flag routes expiry into the same `ui()` branch Windows uses, so there is one
/// definition of what a timer-fired open does.
pub fn tick(ctx: &egui::Context) {
    apply_window_position(ctx);

    if QUIT_REQUESTED.swap(false, Ordering::SeqCst) {
        std::process::exit(0);
    }
    if SETTINGS_REQUESTED.swap(false, Ordering::SeqCst) {
        spawn_settings_process();
    }
    if CONFIG_RELOAD_REQUESTED.swap(false, Ordering::SeqCst) {
        apply_config_reload();
    }

    let popup_visible = POPUP_VISIBLE.load(Ordering::SeqCst);

    if MANUAL_OPEN_REQUESTED.swap(false, Ordering::SeqCst) && !popup_visible {
        // Same edge the Windows tray raises; `ui()` turns it into a
        // `TriggerSource::Manual` open.
        set_tray_clicked();
        reveal(ctx);
        return;
    }

    if popup_visible {
        // The popup is up: the countdown is paused and `ui()` is driving its
        // own 1 Hz repaint, so there is nothing to schedule here.
        return;
    }

    let mut slot = TIMER.lock().unwrap();
    let Some(timer) = *slot else {
        return;
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
        return;
    }

    // Expired. Disarm so one deadline fires once: every close of a
    // timer-fired popup resets the timer (`nudge_state::decide_close`), and
    // that reset is what re-arms us through `set_timer_state`.
    *slot = None;
    drop(slot);

    TIMER_FIRED.store(true, Ordering::SeqCst);
    reveal(ctx);
}

/// Put the popup back on screen. Shared by the timer expiry and every manual
/// trigger, so all four entry points have identical side effects.
///
/// The last `request_repaint` is not redundant: the window is hidden right now,
/// so `ui()` is being skipped and cannot schedule the frame that will consume
/// the flag we just raised.
fn reveal(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
    focus_popup(ctx);
    ctx.request_repaint();
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
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    }
    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
}

// ---------- window placement -----------------------------------------------

/// Move the spotlight window to its spec §1 position, once.
///
/// This cannot happen in the `ViewportBuilder` the way it does on Windows,
/// because the screen size is not knowable before the event loop exists — and
/// the Win32 answer is the wrong shape anyway: `GetSystemMetrics` returns
/// physical pixels while `with_position` takes logical points, so a literal
/// port would double the offset on a Retina display. `monitor_size` comes out
/// of winit already divided by the scale factor, so the arithmetic below is
/// points throughout, which is what `window_position` (and the tests that pin
/// it) assume.
///
/// Costs one frame at winit's default placement on launch, since the first
/// tick is the earliest the monitor is known.
fn apply_window_position(ctx: &egui::Context) {
    if POSITION_APPLIED.load(Ordering::SeqCst) {
        return;
    }
    let Some(monitor) = ctx.input(|i| i.viewport().monitor_size) else {
        // Not known yet (or no monitor at all, under kittest). Try again next
        // tick; the visible launch popup is repainting at 1 Hz.
        return;
    };
    let (x, y) = nudge_state::window_position(
        (monitor.x as u32, monitor.y as u32),
        nudge_state::POPUP_SIZE,
    );
    ctx.send_viewport_cmd(egui::ViewportCommand::OuterPosition(egui::pos2(
        x as f32, y as f32,
    )));
    POSITION_APPLIED.store(true, Ordering::SeqCst);
}

// ---------- status item ----------------------------------------------------

/// Redraw the petal countdown and re-word the tooltip, both only when what
/// they would show has changed — spec §5's "updated once per minute" for the
/// tooltip, and for the icon the whole cost of a status-item repaint, which
/// runs from a 64x64 re-render through to a menu-bar composite.
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

        if ui.last_icon_key != Some(icon_key) {
            let rgba = daisy::render(frame.petals_remaining, frame.drift);
            if let Ok(icon) = tray_icon::Icon::from_rgba(rgba, daisy::ICON_SIZE, daisy::ICON_SIZE) {
                let _ = ui.tray.set_icon(Some(icon));
            }
            ui.last_icon_key = Some(icon_key);
        }

        if ui.last_tooltip_minutes != Some(minutes) {
            let _ = ui
                .tray
                .set_tooltip(Some(nudge_state::tooltip_for_remaining(remaining)));
            ui.last_tooltip_minutes = Some(minutes);
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

/// The `global_hotkey` key code for one of our canonical key tokens. `None`
/// means we don't know how to register this token globally (caller logs and
/// skips) — the macOS counterpart of `tray_bridge::vk_for_key`.
///
/// The token set is exactly what `hotkey::canonicalize_key` produces, so a
/// combo the parser or the settings recorder accepts is a combo this can
/// translate. Whether the *OS* will take it is a separate question:
/// `global_hotkey`'s macOS scancode table stops at F20, so F21..F24 map here
/// and then fail at `register`, loudly.
fn code_for_key(key: &str) -> Option<Code> {
    // `Code`'s own `FromStr` is the mapping — the W3C code names it parses
    // ("KeyA", "Digit0", "F5", "Space", …) are one rename away from our
    // tokens, and enumerating 60 variants by hand would only add a place for
    // them to disagree.
    let name = if key.len() == 1 {
        let ch = key.as_bytes()[0];
        if ch.is_ascii_uppercase() {
            format!("Key{key}")
        } else if ch.is_ascii_digit() {
            format!("Digit{key}")
        } else {
            return None;
        }
    } else if key
        .strip_prefix('F')
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
    {
        key.to_string()
    } else {
        match key {
            "SPACE" => "Space".to_string(),
            "ENTER" => "Enter".to_string(),
            "TAB" => "Tab".to_string(),
            "ESCAPE" => "Escape".to_string(),
            "BACKSPACE" => "Backspace".to_string(),
            _ => return None,
        }
    };
    name.parse().ok()
}

/// Our modifier mask as `global_hotkey`'s. `MOD_WIN` is Cmd here, matching the
/// parser's `Win|Super|Meta|Cmd` aliasing and `hotkey::hotkey_from_egui`'s
/// folding of `mac_cmd` into the same bit.
fn modifiers_to_global(m: u8) -> GlobalModifiers {
    let mut out = GlobalModifiers::empty();
    if m & MY_MOD_CTRL != 0 {
        out |= GlobalModifiers::CONTROL;
    }
    if m & MY_MOD_ALT != 0 {
        out |= GlobalModifiers::ALT;
    }
    if m & MY_MOD_SHIFT != 0 {
        out |= GlobalModifiers::SHIFT;
    }
    if m & MY_MOD_WIN != 0 {
        out |= GlobalModifiers::SUPER;
    }
    out
}

/// Translate a parsed [`Hotkey`] into the binding to register. `None` for a key
/// this backend cannot express.
fn global_hotkey_for(hk: &Hotkey) -> Option<GlobalHotKey> {
    let code = code_for_key(hk.key.as_str())?;
    Some(GlobalHotKey::new(
        Some(modifiers_to_global(hk.modifiers)),
        code,
    ))
}

/// Register `hk`, returning the binding on success. Failure (taken by another
/// app, a key macOS has no scancode for) is logged — we never abort startup
/// over a hotkey.
fn try_register_hotkey(manager: &GlobalHotKeyManager, hk: &Hotkey) -> Option<GlobalHotKey> {
    let Some(binding) = global_hotkey_for(hk) else {
        eprintln!(
            "[nudge] cannot register hotkey: unknown key \"{}\"",
            hk.key.as_str()
        );
        return None;
    };
    match manager.register(binding) {
        Ok(()) => Some(binding),
        Err(e) => {
            eprintln!(
                "[nudge] registering hotkey \"{}\" failed: {e}",
                hotkey::format(hk)
            );
            None
        }
    }
}

// ---------- config live-reload ---------------------------------------------

/// Spawn a second instance of the binary with `--settings`, exactly as the
/// Windows tray menu does. The two processes communicate ONLY through
/// `config.json` (and `~/Library/LaunchAgents` for autostart). Fire-and-forget:
/// a failed spawn is logged and ignored, since settings are editable by hand
/// per spec §5. `--settings` is single-instance in its own right (see
/// [`acquire_settings_singleton`]), so repeat clicks cannot stack windows.
fn spawn_settings_process() {
    match std::env::current_exe() {
        Ok(exe) => {
            if let Err(e) = std::process::Command::new(exe).arg("--settings").spawn() {
                eprintln!("[nudge] could not open settings: {e}");
            }
        }
        Err(e) => eprintln!("[nudge] could not locate own executable: {e}"),
    }
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
            return;
        };

        let (new_cfg, err) = config::load_or_default(&ui.config_path);
        if let Some(e) = err {
            eprintln!("[nudge] config reload: {e} — keeping previous in-memory config");
            return;
        }

        for change in config_watcher::diff(&ui.previous_cfg, &new_cfg) {
            match change {
                ConfigChange::Hotkey { from, to } => {
                    eprintln!("[nudge] config reload: hotkey \"{from}\" → \"{to}\"");
                    let Some(manager) = ui.hotkeys.as_ref() else {
                        eprintln!("[nudge] config reload: no hotkey manager, leaving unbound");
                        continue;
                    };
                    if let Some(previous) = ui.registered.take()
                        && let Err(e) = manager.unregister(previous)
                    {
                        eprintln!("[nudge] config reload: unregistering \"{from}\" failed: {e}");
                    }
                    let (parsed, invalid) = new_cfg.resolved_hotkey();
                    if invalid {
                        eprintln!(
                            "[nudge] config reload: hotkey \"{to}\" is unparseable, leaving unbound"
                        );
                    } else {
                        ui.registered = try_register_hotkey(manager, &parsed);
                    }
                }
                ConfigChange::Interval => eprintln!(
                    "[nudge] config reload: default_interval_minutes {} → {} \
                     (no live effect; applies at next startup)",
                    ui.previous_cfg.default_interval_minutes, new_cfg.default_interval_minutes
                ),
                ConfigChange::Autostart => eprintln!(
                    "[nudge] config reload: autostart {} → {} \
                     (settings process owns the LaunchAgent)",
                    ui.previous_cfg.autostart, new_cfg.autostart
                ),
            }
        }

        ui.previous_cfg = new_cfg;
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // === hotkey → global_hotkey ==============================================
    //
    // The same combos `hotkey.rs`'s own tests round-trip, carried one step
    // further: parse the label a user can write, then assert the binding this
    // backend would hand to Carbon. A key the parser accepts and this cannot
    // translate is a hotkey that silently never fires.

    fn binding(label: &str) -> GlobalHotKey {
        let hk = hotkey::parse(label).expect("label must parse");
        global_hotkey_for(&hk).unwrap_or_else(|| panic!("no binding for {label}"))
    }

    #[test]
    fn default_hotkey_maps_to_ctrl_shift_space() {
        let hk = hotkey::default_hotkey();
        let b = global_hotkey_for(&hk).expect("the default must be registrable");
        assert_eq!(b.key, Code::Space);
        assert_eq!(
            b.mods,
            GlobalModifiers::CONTROL | GlobalModifiers::SHIFT,
            "Ctrl+Shift, and nothing else"
        );
    }

    #[test]
    fn letters_map_to_key_codes() {
        assert_eq!(binding("Ctrl+A").key, Code::KeyA);
        assert_eq!(binding("Ctrl+J").key, Code::KeyJ);
        assert_eq!(binding("Ctrl+Z").key, Code::KeyZ);
    }

    #[test]
    fn digits_map_to_digit_codes() {
        assert_eq!(binding("Ctrl+0").key, Code::Digit0);
        assert_eq!(binding("Ctrl+7").key, Code::Digit7);
        assert_eq!(binding("Ctrl+9").key, Code::Digit9);
    }

    #[test]
    fn f_keys_map_across_the_whole_parser_range() {
        // F1..F24 is what `canonicalize_key` accepts, so all 24 must translate
        // — even the ones macOS has no scancode for. Those fail at
        // `register`, with a message; they must not fail here, silently.
        for n in 1..=24u8 {
            let b = binding(&format!("Ctrl+F{n}"));
            assert_eq!(
                b.key,
                format!("F{n}").parse::<Code>().unwrap(),
                "F{n} must map"
            );
        }
    }

    #[test]
    fn named_keys_map() {
        assert_eq!(binding("Ctrl+Space").key, Code::Space);
        assert_eq!(binding("Ctrl+Enter").key, Code::Enter);
        assert_eq!(binding("Ctrl+Return").key, Code::Enter);
        assert_eq!(binding("Ctrl+Tab").key, Code::Tab);
        assert_eq!(binding("Ctrl+Esc").key, Code::Escape);
        assert_eq!(binding("Ctrl+Escape").key, Code::Escape);
        assert_eq!(binding("Ctrl+Backspace").key, Code::Backspace);
    }

    #[test]
    fn every_key_the_parser_accepts_has_a_binding() {
        // The invariant that matters: `hotkey::parse` and `code_for_key` share
        // one key set. Anything the config file can legally name has to reach
        // the OS.
        let mut labels: Vec<String> = Vec::new();
        for ch in ('A'..='Z').chain('0'..='9') {
            labels.push(format!("Ctrl+{ch}"));
        }
        for n in 1..=24u8 {
            labels.push(format!("Ctrl+F{n}"));
        }
        for named in [
            "Space",
            "Enter",
            "Return",
            "Tab",
            "Esc",
            "Escape",
            "Backspace",
        ] {
            labels.push(format!("Ctrl+{named}"));
        }
        for label in labels {
            let hk = hotkey::parse(&label).expect("label must parse");
            assert!(
                global_hotkey_for(&hk).is_some(),
                "{label} parses but has no binding"
            );
        }
    }

    #[test]
    fn modifiers_map_one_for_one() {
        assert_eq!(modifiers_to_global(0), GlobalModifiers::empty());
        assert_eq!(modifiers_to_global(MY_MOD_CTRL), GlobalModifiers::CONTROL);
        assert_eq!(modifiers_to_global(MY_MOD_ALT), GlobalModifiers::ALT);
        assert_eq!(modifiers_to_global(MY_MOD_SHIFT), GlobalModifiers::SHIFT);
        // Cmd, not Control: the parser aliases Win/Super/Meta/Cmd onto one bit
        // and this is the platform where that bit is the Command key.
        assert_eq!(modifiers_to_global(MY_MOD_WIN), GlobalModifiers::SUPER);
    }

    #[test]
    fn all_four_modifiers_compose() {
        let b = binding("Ctrl+Alt+Shift+Cmd+J");
        assert_eq!(
            b.mods,
            GlobalModifiers::CONTROL
                | GlobalModifiers::ALT
                | GlobalModifiers::SHIFT
                | GlobalModifiers::SUPER
        );
        assert_eq!(b.key, Code::KeyJ);
    }

    #[test]
    fn unknown_tokens_have_no_binding() {
        // Not reachable through `parse` — these are the shapes `code_for_key`
        // must reject on its own, so a future widening of the parser cannot
        // quietly produce an unregistrable binding.
        // "F" is absent on purpose: it is the letter F, a key the parser does
        // accept, and it must map — see `letters_map_to_key_codes`.
        for token in ["", "+", "HOME", "ARROWUP", "F0", "F99", "AA", "мама"] {
            assert!(code_for_key(token).is_none(), "{token:?} must not map");
        }
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
        // 100ms into a petal drop, inside the 250ms drift window.
        let remaining = Duration::from_millis(549_900);
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

    /// The middle link of the manual-open path: a handler raises a flag from
    /// outside any frame, and the *next* tick has to turn it into the
    /// `TRAY_CLICKED` edge `ui()` consumes. If `tick` dropped it, a status-item
    /// click while the popup is hidden — the only state the status item exists
    /// for — would do nothing at all.
    ///
    /// One test rather than two because the flags are process globals: two
    /// tests asserting on them would race under libtest's default parallelism.
    #[test]
    fn tick_turns_a_handler_flag_into_the_edge_the_app_reads() {
        let ctx = egui::Context::default();
        let _ = take_tray_clicked();

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

    // === window placement ====================================================

    #[test]
    fn position_is_computed_in_points_not_pixels() {
        // The Retina trap: `monitor_size` is already logical, so a 1470x956
        // built-in display must produce the 1470x956 answer — not the 2940x1912
        // one a `GetSystemMetrics`-shaped port would compute.
        let (x, y) = nudge_state::window_position((1470, 956), nudge_state::POPUP_SIZE);
        assert_eq!((x, y), (475, 239));
        let (x2, y2) = nudge_state::window_position((2940, 1912), nudge_state::POPUP_SIZE);
        assert_ne!(
            (x, y),
            (x2, y2),
            "if these agreed the test could not see the doubling it exists to catch"
        );
    }
}
