/// Bridge between tray icon event handlers (run on Windows message thread)
/// and the eframe update loop. Event handlers set flags + restore window;
/// update() checks flags once it resumes.
use std::sync::atomic::{AtomicBool, AtomicIsize, AtomicU64, Ordering};

static HWND_STORE: AtomicIsize = AtomicIsize::new(0);
static TRAY_CLICKED: AtomicBool = AtomicBool::new(false);
static EXIT_REQUESTED: AtomicBool = AtomicBool::new(false);
static TIMER_FIRED: AtomicBool = AtomicBool::new(false);
static TIMER_GENERATION: AtomicU64 = AtomicU64::new(0);

pub fn store_hwnd(hwnd: isize) {
    HWND_STORE.store(hwnd, Ordering::SeqCst);
}

pub fn load_hwnd() -> Option<isize> {
    let val = HWND_STORE.load(Ordering::SeqCst);
    if val != 0 { Some(val) } else { None }
}

pub fn set_tray_clicked() {
    TRAY_CLICKED.store(true, Ordering::SeqCst);
}

pub fn take_tray_clicked() -> bool {
    TRAY_CLICKED.swap(false, Ordering::SeqCst)
}

pub fn set_exit_requested() {
    EXIT_REQUESTED.store(true, Ordering::SeqCst);
}

pub fn is_exit_requested() -> bool {
    EXIT_REQUESTED.load(Ordering::SeqCst)
}

pub fn take_timer_fired() -> bool {
    TIMER_FIRED.swap(false, Ordering::SeqCst)
}

/// Spawn a background thread that sleeps for `duration` then restores window.
/// Each call increments a generation counter — stale threads become no-ops.
pub fn schedule_timer_wakeup(duration: std::time::Duration) {
    let generation = TIMER_GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    std::thread::spawn(move || {
        std::thread::sleep(duration);
        // Only fire if no newer timer was scheduled
        if TIMER_GENERATION.load(Ordering::SeqCst) == generation {
            TIMER_FIRED.store(true, Ordering::SeqCst);
            if let Some(hwnd_val) = load_hwnd() {
                unsafe {
                    use windows::Win32::Foundation::HWND;
                    use windows::Win32::UI::WindowsAndMessaging::*;
                    let h = HWND(hwnd_val as *mut _);
                    let _ = ShowWindow(h, SW_RESTORE);
                    let _ = SetForegroundWindow(h);
                }
            }
        }
    });
}
