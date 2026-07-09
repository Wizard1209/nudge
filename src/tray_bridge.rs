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

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, WPARAM};
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, MOD_WIN, RegisterHotKey,
    UnregisterHotKey,
};
use windows::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, DispatchMessageW, GetForegroundWindow, GetWindowThreadProcessId, MSG,
    MWMO_INPUTAVAILABLE, MsgWaitForMultipleObjectsEx, PM_REMOVE, PeekMessageW, PostThreadMessageW,
    QS_ALLINPUT, SW_RESTORE, SetForegroundWindow, ShowWindow, TranslateMessage, WM_HOTKEY, WM_USER,
};

use crate::config;
use crate::config_watcher::{self, ConfigChange};
use crate::daisy;
use crate::hotkey::{
    self, Hotkey, MOD_ALT as MY_MOD_ALT, MOD_CTRL, MOD_SHIFT as MY_MOD_SHIFT, MOD_WIN as MY_MOD_WIN,
};
use crate::nudge_state;

// ---------- shared state ---------------------------------------------------

/// Handle of the eframe popup window so we can `ShowWindow` it from the
/// tray thread when the timer expires.
static HWND_STORE: AtomicIsize = AtomicIsize::new(0);

/// Edge flags polled by `NudgeApp::update()`.
static TRAY_CLICKED: AtomicBool = AtomicBool::new(false);
static TIMER_FIRED: AtomicBool = AtomicBool::new(false);

/// Set by the config_watcher callback (off the tray thread) when
/// `config.json` changes on disk. The tray thread picks this up on its
/// next wake, re-reads the file, and applies any live-meaningful diff
/// (currently: re-register the hotkey when it differs). Stays an
/// AtomicBool rather than a counter — N concurrent saves all collapse
/// to "re-read once", which is the correct behaviour.
static CONFIG_RELOAD_REQUESTED: AtomicBool = AtomicBool::new(false);

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

/// Signal that `config.json` changed on disk and the tray thread should
/// re-read + diff it on its next iteration. Called from the
/// `config_watcher` callback, off any tray thread. Idempotent: setting
/// the flag twice before the tray sees it collapses to one re-read.
pub fn request_config_reload() {
    CONFIG_RELOAD_REQUESTED.store(true, Ordering::SeqCst);
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
///
/// `hotkey` (when present) is registered on the tray thread via
/// `RegisterHotKey`. It must be registered on the same thread that pumps
/// the message queue, otherwise WM_HOTKEY is delivered to a queue we
/// aren't reading. None disables the hotkey for this run (e.g. an invalid
/// config label).
///
/// `config_path` and `initial_config` are the launch-time snapshot. The
/// tray thread keeps `initial_config` as its `previous_cfg` baseline so
/// that when the watcher fires it can diff the freshly-re-read file
/// against what's currently in effect (e.g. only re-register the
/// hotkey when the *parsed* string differs, not on every save). The
/// path lets the tray thread re-read the file from the same source the
/// settings process writes to — including any `--config` override.
pub fn spawn_tray_thread(
    hotkey: Option<Hotkey>,
    config_path: PathBuf,
    initial_config: config::Config,
) {
    thread::Builder::new()
        .name("nudge-tray".into())
        .spawn(move || tray_thread_main(hotkey, config_path, initial_config))
        .expect("failed to spawn tray thread");
}

/// ID we pass to RegisterHotKey. Single hotkey for now; if we ever add more
/// (e.g. "pause", "skip"), bump to distinct constants.
const HOTKEY_ID_SHOW_POPUP: i32 = 1;

/// Win32 VK code for a parsed key token. None means we don't know how to
/// register this token globally (caller logs and skips).
fn vk_for_key(key: &str) -> Option<u32> {
    if key.len() == 1 {
        let ch = key.as_bytes()[0];
        if ch.is_ascii_uppercase() || ch.is_ascii_digit() {
            // VK_A..VK_Z and VK_0..VK_9 use the ASCII codes directly.
            return Some(ch as u32);
        }
    }
    if let Some(rest) = key.strip_prefix('F')
        && let Ok(n) = rest.parse::<u8>()
        && (1..=24).contains(&n)
    {
        // VK_F1 = 0x70 … VK_F24 = 0x87
        return Some(0x70 + (n as u32) - 1);
    }
    match key {
        "SPACE" => Some(0x20),     // VK_SPACE
        "ENTER" => Some(0x0D),     // VK_RETURN
        "TAB" => Some(0x09),       // VK_TAB
        "ESCAPE" => Some(0x1B),    // VK_ESCAPE
        "BACKSPACE" => Some(0x08), // VK_BACK
        _ => None,
    }
}

fn modifiers_to_win32(m: u8) -> HOT_KEY_MODIFIERS {
    let mut out = MOD_NOREPEAT;
    if m & MOD_CTRL != 0 {
        out |= MOD_CONTROL;
    }
    if m & MY_MOD_ALT != 0 {
        out |= MOD_ALT;
    }
    if m & MY_MOD_SHIFT != 0 {
        out |= MOD_SHIFT;
    }
    if m & MY_MOD_WIN != 0 {
        out |= MOD_WIN;
    }
    out
}

/// Register the configured global hotkey on the current thread. Returns
/// true on success. Failure (taken by another app, unknown key) is logged
/// to stderr — we never abort startup over a hotkey.
fn try_register_hotkey(hk: &Hotkey) -> bool {
    let Some(vk) = vk_for_key(hk.key.as_str()) else {
        eprintln!(
            "[nudge] cannot register hotkey: unknown key \"{}\"",
            hk.key.as_str()
        );
        return false;
    };
    let mods = modifiers_to_win32(hk.modifiers);
    let ok = unsafe { RegisterHotKey(None, HOTKEY_ID_SHOW_POPUP, mods, vk).is_ok() };
    if !ok {
        eprintln!(
            "[nudge] RegisterHotKey failed for \"{}\" — probably bound by another app",
            hotkey::format(hk)
        );
    }
    ok
}

// ---------- the tray thread itself -----------------------------------------

fn tray_thread_main(hotkey: Option<Hotkey>, config_path: PathBuf, initial_config: config::Config) {
    unsafe {
        TRAY_THREAD_ID.store(GetCurrentThreadId(), Ordering::SeqCst);
    }

    // Register global hotkey on this thread so WM_HOTKEY is posted into this
    // message queue. None / failure both just leave the app hotkey-less for
    // this run — the tray icon still works as the manual-open path.
    // Failure is logged by try_register_hotkey and is non-fatal.
    if let Some(hk) = &hotkey {
        try_register_hotkey(hk);
    }

    // Live-reload bookkeeping: the previous Config so each reload can
    // diff against what we actually applied last, and a flag tracking
    // whether the hotkey is currently registered (so we know whether
    // to call UnregisterHotKey before re-registering).
    let mut previous_cfg = initial_config;
    let mut hotkey_registered = hotkey.is_some();

    use tray_icon::TrayIconBuilder;
    use tray_icon::menu::{Menu, MenuItem};

    let menu = Menu::new();
    let [show_label, settings_label, quit_label] = nudge_state::TRAY_MENU_LABELS;
    let show_item = MenuItem::new(show_label, true, None);
    let settings_item = MenuItem::new(settings_label, true, None);
    let quit_item = MenuItem::new(quit_label, true, None);
    menu.append(&show_item).unwrap();
    menu.append(&settings_item).unwrap();
    menu.append(&quit_item).unwrap();
    let show_id = show_item.id().clone();
    let settings_id = settings_item.id().clone();
    let quit_id = quit_item.id().clone();

    let initial_rgba = daisy::render(daisy::PETAL_COUNT, None);
    let initial_icon = tray_icon::Icon::from_rgba(initial_rgba, daisy::ICON_SIZE, daisy::ICON_SIZE)
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
            if event.id == settings_id {
                // Spawn a second instance of nudge.exe with --settings.
                // The two processes communicate ONLY through config.json
                // (and the registry for autostart). We fire-and-forget; a
                // failed spawn is logged-and-ignored — settings are also
                // editable by hand per spec §5.
                if let Ok(exe) = std::env::current_exe() {
                    let _ = std::process::Command::new(exe).arg("--settings").spawn();
                }
            }
        },
    ));

    // Hold these so they aren't dropped — menu only keeps weak refs.
    let _menu_items = (show_item, settings_item, quit_item);

    // We dedupe identical icon updates so set_icon isn't called every loop.
    // State key: (petals_remaining, drift_progress_x100, drift_active).
    let mut last_state_key: (u8, u16, bool) = (255, 0, false);
    // Dedupe tooltip updates the same way — only call set_tooltip when the
    // rendered minute number changes (spec §5: "updated once per minute").
    // `None` forces a refresh on the first loop iteration after a new timer.
    let mut last_tooltip_minutes: Option<u64> = None;
    // Latch so a single timer instance fires the popup at most once.
    let mut fired_for_deadline: Option<Instant> = None;

    loop {
        drain_messages();

        // Apply pending config-file reloads before we either park
        // (popup visible) or animate. Doing it here means a reload that
        // arrives mid-popup gets applied as soon as the popup closes —
        // we never silently drop a request, even though we don't fight
        // the popup for attention while it's up.
        if CONFIG_RELOAD_REQUESTED.swap(false, Ordering::SeqCst) {
            apply_config_reload(&config_path, &mut previous_cfg, &mut hotkey_registered);
        }

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

        // Spec §5 timeline math lives in `daisy::frame_at` (pure + tested);
        // the tray thread keeps only the wall-clock measurement above and the
        // Win32 I/O below.
        let frame = daisy::frame_at(timer.duration, elapsed);
        let petals_remaining = frame.petals_remaining;
        let drift = frame.drift;

        let drift_scaled = drift.map(|d| (d.progress * 100.0) as u16).unwrap_or(0);
        let key = (petals_remaining, drift_scaled, drift.is_some());
        if key != last_state_key {
            let rgba = daisy::render(petals_remaining, drift);
            if let Ok(icon) = tray_icon::Icon::from_rgba(rgba, daisy::ICON_SIZE, daisy::ICON_SIZE) {
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

        // Time to fire the popup? `frame.should_fire` waits for the very last
        // drift to finish so the user actually sees the final petal fall
        // before the popup takes focus.
        if frame.should_fire && fired_for_deadline != Some(timer.deadline) {
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
            let to_next = frame.petal_duration.saturating_sub(frame.time_since_drop);
            (to_next.as_millis() as u32).min(60_000)
        };

        wait_for_message(next_wake_ms);
    }
}

/// Re-read `config_path` and apply any live-meaningful diff against
/// `previous_cfg`. Only the hotkey has live UI effect on the running
/// app today (see module doc + `config_watcher` docs); the interval
/// and autostart fields are updated in the in-memory record so the
/// next diff is computed against truth, but they don't trigger any
/// syscalls here.
///
/// A malformed file is tolerated the same way `load_or_default` does
/// it: log to stderr, fall back to defaults, keep the previous in-
/// memory record. The watcher must not be the thing that kills the
/// app over a typo in the user's config.
fn apply_config_reload(
    config_path: &std::path::Path,
    previous_cfg: &mut config::Config,
    hotkey_registered: &mut bool,
) {
    let (new_cfg, err) = config::load_or_default(config_path);
    if let Some(e) = err {
        eprintln!("[nudge] config reload: {e} — keeping previous in-memory config");
        return;
    }

    let changes = config_watcher::diff(previous_cfg, &new_cfg);
    if changes.is_empty() {
        return;
    }

    for change in &changes {
        match change {
            ConfigChange::Hotkey { from, to } => {
                eprintln!("[nudge] config reload: hotkey \"{from}\" → \"{to}\"");
                // Unregister the previous binding (best-effort — if it
                // wasn't actually registered, Win32 returns FALSE and
                // we ignore it; the next try_register_hotkey is the
                // operative call).
                if *hotkey_registered {
                    unsafe {
                        let _ = UnregisterHotKey(None, HOTKEY_ID_SHOW_POPUP);
                    }
                    *hotkey_registered = false;
                }
                let (parsed, invalid) = new_cfg.resolved_hotkey();
                if invalid {
                    eprintln!(
                        "[nudge] config reload: hotkey \"{to}\" is unparseable, leaving unbound"
                    );
                } else {
                    *hotkey_registered = try_register_hotkey(&parsed);
                }
            }
            ConfigChange::Interval => {
                // Recorded only: the popup captured the launch-time
                // default and the user owns the field afterwards.
                eprintln!(
                    "[nudge] config reload: default_interval_minutes \
                     {} → {} (no live effect; applies at next startup)",
                    previous_cfg.default_interval_minutes, new_cfg.default_interval_minutes
                );
            }
            ConfigChange::Autostart => {
                // Settings process owns the registry write
                // transactionally; we just refresh our cached copy.
                eprintln!(
                    "[nudge] config reload: autostart {} → {} (settings process owns registry)",
                    previous_cfg.autostart, new_cfg.autostart
                );
            }
        }
    }

    *previous_cfg = new_cfg;
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
            // WM_HOTKEY is posted to the thread message queue (no hwnd), so
            // we must handle it before DispatchMessageW — Dispatch would
            // drop it for lack of a target window. Same UX path as a tray
            // left-click: pop the popup as a Manual trigger.
            if msg.message == WM_HOTKEY && msg.wParam.0 == HOTKEY_ID_SHOW_POPUP as usize {
                set_tray_clicked();
                wake_eframe();
                continue;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}

fn wait_for_message(timeout_ms: u32) {
    unsafe {
        let _ = MsgWaitForMultipleObjectsEx(None, timeout_ms, QS_ALLINPUT, MWMO_INPUTAVAILABLE);
    }
}
