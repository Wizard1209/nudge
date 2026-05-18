//! Tray icon owned by a dedicated thread.
//!
//! Why a separate thread? `tray_icon::TrayIcon` is `!Send` (wraps
//! `Rc<RefCell<…>>`), so the tray has to live entirely on one thread. The
//! main thread runs eframe, which `SW_HIDE`s its window between nudges —
//! and a hidden, layered, transparent window does not get `update()`
//! callbacks. Driving icon animation from `update()` therefore drops most
//! frames. Discord and similar apps avoid this by giving the tray its own
//! thread with its own message pump; we do the same here.
//!
//! The tray thread:
//!   - Owns the `TrayIcon` and the menu items.
//!   - Runs a `MsgWaitForMultipleObjectsEx`-driven loop that interleaves
//!     message dispatch with animation frames.
//!   - Reads timer state out of `ANIMATOR_TIMER` and popup state out of
//!     `POPUP_VISIBLE`, both updated by the eframe thread.
//!   - Wakes from a long wait via `PostThreadMessageW` whenever eframe
//!     changes state.
//!   - Fires the popup when the timer expires (sets `TIMER_FIRED` +
//!     `ShowWindow(SW_RESTORE)` on eframe's HWND).

use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, DispatchMessageW, GetForegroundWindow, GetWindowThreadProcessId, MSG,
    MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_REMOVE, PeekMessageW, PostThreadMessageW,
    QS_ALLINPUT, SW_RESTORE, SetForegroundWindow, ShowWindow, TranslateMessage, WM_USER,
};

use crate::daisy;
use crate::nudge_state;

// ---------- shared state ---------------------------------------------------

/// Handle of the eframe popup window so we can `ShowWindow` it from the
/// tray thread when the timer expires.
static HWND_STORE: AtomicIsize = AtomicIsize::new(0);

/// Edge flags polled by `NudgeApp::update()`.
static TRAY_CLICKED: AtomicBool = AtomicBool::new(false);
static TIMER_FIRED: AtomicBool = AtomicBool::new(false);

/// `true` while the popup is on screen. The tray thread treats this as
/// "freeze animation": clear out any pending repaints and wait until eframe
/// flips it back to `false`.
static POPUP_VISIBLE: AtomicBool = AtomicBool::new(true);

/// OS thread id of the tray thread — used by `PostThreadMessageW` to wake
/// the pump when state changes from the eframe side.
static TRAY_THREAD_ID: AtomicU32 = AtomicU32::new(0);

#[derive(Clone, Copy)]
struct AnimatorTimer {
    deadline: Instant,
    duration: Duration,
}

static ANIMATOR_TIMER: Mutex<Option<AnimatorTimer>> = Mutex::new(None);

/// Custom message we post to the tray thread purely to wake the pump out
/// of `MsgWaitForMultipleObjectsEx`. Value is opaque — we drain and ignore.
const WM_TRAY_WAKE: u32 = WM_USER + 1;

// ---------- public API used by main / NudgeApp -----------------------------

pub fn store_hwnd(hwnd: isize) {
    HWND_STORE.store(hwnd, Ordering::SeqCst);
}

pub fn load_hwnd() -> Option<isize> {
    let val = HWND_STORE.load(Ordering::SeqCst);
    if val != 0 { Some(val) } else { None }
}

/// Force `hwnd` to become the actual foreground window (visible AND focused).
///
/// `SetForegroundWindow` alone is restricted by Win32: it silently no-ops
/// when the caller isn't already the foreground process or hasn't received
/// recent input. For a timer-fired popup that's exactly the failure case —
/// the window appears layered-on-top but gets no input focus, so click-outside
/// and focus-loss detection never see anything to fire on.
///
/// Workaround: attach the window's owning thread's input queue to the
/// current foreground thread's. While attached, Win32 treats SetForegroundWindow
/// as if it came from the foreground process and grants the activation.
pub fn force_foreground(hwnd: HWND) {
    unsafe {
        let _ = ShowWindow(hwnd, SW_RESTORE);

        let fg = GetForegroundWindow();
        let fg_tid = if fg.0.is_null() {
            0
        } else {
            GetWindowThreadProcessId(fg, None)
        };
        let window_tid = GetWindowThreadProcessId(hwnd, None);

        let attached = fg_tid != 0
            && window_tid != 0
            && fg_tid != window_tid
            && AttachThreadInput(window_tid, fg_tid, true).as_bool();

        let _ = BringWindowToTop(hwnd);
        let _ = SetForegroundWindow(hwnd);

        if attached {
            let _ = AttachThreadInput(window_tid, fg_tid, false);
        }
    }
}

pub fn set_tray_clicked() {
    TRAY_CLICKED.store(true, Ordering::SeqCst);
}

pub fn take_tray_clicked() -> bool {
    TRAY_CLICKED.swap(false, Ordering::SeqCst)
}

pub fn take_timer_fired() -> bool {
    TIMER_FIRED.swap(false, Ordering::SeqCst)
}

/// Push a fresh deadline / duration to the animator. Called from
/// `NudgeApp::hide_popup` whenever the timer is reset.
pub fn set_timer_state(deadline: Instant, duration: Duration) {
    *ANIMATOR_TIMER.lock().unwrap() = Some(AnimatorTimer { deadline, duration });
    wake_tray_thread();
}

/// Mirror `popup_visible` into the tray thread so it can park instead of
/// repainting while the user is interacting with the form.
pub fn set_popup_visible(visible: bool) {
    POPUP_VISIBLE.store(visible, Ordering::SeqCst);
    wake_tray_thread();
}

fn wake_tray_thread() {
    let tid = TRAY_THREAD_ID.load(Ordering::SeqCst);
    if tid != 0 {
        unsafe {
            let _ = PostThreadMessageW(tid, WM_TRAY_WAKE, WPARAM(0), LPARAM(0));
        }
    }
}

/// Spawn the long-lived tray thread. Call once at startup, before
/// `eframe::run_native`. The thread creates the tray icon, sets the
/// global tray / menu event handlers, and runs the animation loop.
pub fn spawn_tray_thread() {
    thread::Builder::new()
        .name("nudge-tray".into())
        .spawn(tray_thread_main)
        .expect("failed to spawn tray thread");
}

// ---------- the tray thread itself -----------------------------------------

fn tray_thread_main() {
    unsafe {
        TRAY_THREAD_ID.store(GetCurrentThreadId(), Ordering::SeqCst);
    }

    use tray_icon::TrayIconBuilder;
    use tray_icon::menu::{Menu, MenuItem};

    let menu = Menu::new();
    let [show_label, quit_label] = nudge_state::TRAY_MENU_LABELS;
    let show_item = MenuItem::new(show_label, true, None);
    let quit_item = MenuItem::new(quit_label, true, None);
    menu.append(&show_item).unwrap();
    menu.append(&quit_item).unwrap();
    let show_id = show_item.id().clone();
    let quit_id = quit_item.id().clone();

    let initial_rgba = daisy::render(daisy::PETAL_COUNT, None);
    let initial_icon =
        tray_icon::Icon::from_rgba(initial_rgba, daisy::ICON_SIZE, daisy::ICON_SIZE)
            .expect("failed to create initial tray icon");
    // Tooltip will be refreshed by the loop below as soon as eframe pushes
    // a timer via set_timer_state. Until then the popup is visible (first
    // launch), so the user never sees this placeholder anyway.
    let initial_tooltip = "Nudge";

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip(initial_tooltip)
        .with_icon(initial_icon)
        .build()
        .expect("failed to build tray icon");

    // tray-icon installs ONE global event handler. We register both here so
    // they fire on this thread's message pump.
    tray_icon::TrayIconEvent::set_event_handler(Some(|event| {
        if matches!(
            event,
            tray_icon::TrayIconEvent::Click {
                button: tray_icon::MouseButton::Left,
                button_state: tray_icon::MouseButtonState::Up,
                ..
            }
        ) {
            set_tray_clicked();
            wake_eframe();
        }
    }));

    tray_icon::menu::MenuEvent::set_event_handler(Some(
        move |event: tray_icon::menu::MenuEvent| {
            if event.id == quit_id {
                std::process::exit(0);
            }
            if event.id == show_id {
                set_tray_clicked();
                wake_eframe();
            }
        },
    ));

    // Hold these so they aren't dropped — menu only keeps weak refs.
    let _menu_items = (show_item, quit_item);

    // We dedupe identical icon updates so set_icon isn't called every loop.
    // State key: (petals_remaining, drift_progress_x100, drift_active).
    let mut last_state_key: (u8, u16, bool) = (255, 0, false);
    // Dedupe tooltip updates the same way — only call set_tooltip when the
    // rendered minute number changes (spec §5: "обновляется раз в минуту").
    // `None` forces a refresh on the first loop iteration after a new timer.
    let mut last_tooltip_minutes: Option<u64> = None;
    // Latch so a single timer instance fires the popup at most once.
    let mut fired_for_deadline: Option<Instant> = None;

    loop {
        drain_messages();

        if POPUP_VISIBLE.load(Ordering::SeqCst) {
            // Popup is up. Don't fight it for attention; idle until eframe
            // tells us things have changed.
            wait_for_message(u32::MAX);
            continue;
        }

        let timer = match *ANIMATOR_TIMER.lock().unwrap() {
            Some(t) => t,
            None => {
                wait_for_message(u32::MAX);
                continue;
            }
        };

        let now = Instant::now();
        // Let elapsed keep growing past `duration` so the final drift
        // animation can finish before we settle on the bare-center frame.
        let elapsed = if now >= timer.deadline {
            timer.duration + now.duration_since(timer.deadline)
        } else {
            timer
                .duration
                .saturating_sub(timer.deadline.duration_since(now))
        };

        let petal_count_u64 = daisy::PETAL_COUNT as u64;
        let petal_dur_nanos = (timer.duration.as_nanos() as u64).max(1) / petal_count_u64;
        let petal_dur = Duration::from_nanos(petal_dur_nanos);
        let elapsed_nanos = elapsed.as_nanos() as u64;
        let raw_drops = elapsed_nanos / petal_dur_nanos;
        let petals_dropped = raw_drops.min(petal_count_u64) as u8;
        let petals_remaining = daisy::PETAL_COUNT - petals_dropped;
        let time_since_drop = Duration::from_nanos(
            elapsed_nanos - (petals_dropped as u64) * petal_dur_nanos,
        );
        // Drift gets at most half a petal lifetime, so it can't run into
        // the next drop even at silly-short intervals.
        let drift_dur = Duration::from_millis(250).min(petal_dur / 2);

        let drift = if petals_dropped > 0 && time_since_drop < drift_dur {
            let progress = (time_since_drop.as_secs_f32()
                / drift_dur.as_secs_f32())
            .clamp(0.0, 1.0);
            Some(daisy::DriftState {
                petal_index: petals_dropped - 1,
                progress,
            })
        } else {
            None
        };

        let drift_scaled = drift.map(|d| (d.progress * 100.0) as u16).unwrap_or(0);
        let key = (petals_remaining, drift_scaled, drift.is_some());
        if key != last_state_key {
            let rgba = daisy::render(petals_remaining, drift);
            if let Ok(icon) =
                tray_icon::Icon::from_rgba(rgba, daisy::ICON_SIZE, daisy::ICON_SIZE)
            {
                let _ = tray.set_icon(Some(icon));
            }
            last_state_key = key;
        }

        // Tooltip — refresh only when the displayed minute number changes.
        // `remaining` saturates at zero past the deadline, so `tooltip_for_remaining`
        // produces "now" during the drift-to-fire window.
        let remaining = timer.deadline.saturating_duration_since(now);
        let mins_now = nudge_state::displayed_minutes(remaining);
        if last_tooltip_minutes != Some(mins_now) {
            let text = nudge_state::tooltip_for_remaining(remaining);
            let _ = tray.set_tooltip(Some(&text));
            last_tooltip_minutes = Some(mins_now);
        }

        // Time to fire the popup? Wait for the very last drift to finish so
        // the user actually sees the final petal fall before the popup
        // takes focus.
        let final_drift_done = elapsed >= timer.duration + drift_dur;
        if final_drift_done && fired_for_deadline != Some(timer.deadline) {
            fired_for_deadline = Some(timer.deadline);
            TIMER_FIRED.store(true, Ordering::SeqCst);
            if let Some(hwnd_val) = load_hwnd() {
                force_foreground(HWND(hwnd_val as *mut _));
            }
            // Park; eframe will flip POPUP_VISIBLE shortly and wake us.
            wait_for_message(u32::MAX);
            continue;
        }

        let next_wake_ms: u32 = if drift.is_some() {
            33 // ~30 Hz during drift
        } else if elapsed >= timer.duration {
            // Past expiry but pre-fire — re-check soon.
            10
        } else {
            // Until the next petal drop, clamped to a minute as a sanity
            // backstop in case the system clock jumps.
            let to_next = petal_dur.saturating_sub(time_since_drop);
            (to_next.as_millis() as u32).min(60_000)
        };

        wait_for_message(next_wake_ms);
    }
}

fn wake_eframe() {
    if let Some(hwnd_val) = load_hwnd() {
        force_foreground(HWND(hwnd_val as *mut _));
    }
}

fn drain_messages() {
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn wait_for_message(timeout_ms: u32) {
    unsafe {
        let _ = MsgWaitForMultipleObjectsEx(
            None,
            timeout_ms,
            QS_ALLINPUT,
            MWMO_INPUTAVAILABLE,
        );
    }
}
