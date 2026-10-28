//! Windows backend: mostly `tray_bridge`, re-exported.
//!
//! The tray thread there owns the timer, the hotkey and the popup wakeup.
//! What lives here is what the seam needs on top: adapters for the entry
//! points whose signatures carried `HWND`, and the screen the popup is laid
//! out on (spec §1), which `tray_bridge::force_foreground` picks at show time.

use std::sync::Mutex;

use ::windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, TRUE};
use ::windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    GetCursorPos, GetForegroundWindow, GetWindowRect, MONITORINFOF_PRIMARY,
};
use ::windows::core::BOOL;
use eframe::egui;

use crate::config;
use crate::hotkey::Hotkey;
use crate::sprite::ScreenGeometry;

pub use crate::tray_bridge::{
    request_config_reload, set_popup_visible, set_timer_state, take_pending_sprite,
    take_timer_fired, take_tray_clicked,
};

/// The raise side of `take_tray_clicked`. No caller goes through the seam for
/// it today — on Windows the tray thread's own event handlers set the flag
/// directly, and macOS has no status item yet — but it is half of a pair, and
/// dropping it would leave the macOS tray with nothing to call when it lands.
#[allow(unused_imports)]
pub use crate::tray_bridge::set_tray_clicked;

/// Start the OS-integration machinery: here, the tray thread, which also
/// owns the global hotkey and the config-reload diff.
pub fn start(hotkey: Option<Hotkey>, config_path: std::path::PathBuf, initial: config::Config) {
    crate::tray_bridge::spawn_tray_thread(hotkey, config_path, initial);
}

/// No-op: `start` already built everything on its own thread, and none of it
/// needs the event loop. The macOS arm is the one with work here — its status
/// item and hotkey cannot exist until the main thread is the event loop's.
pub fn start_on_event_loop(_ctx: &egui::Context) {}

/// `tray_bridge::store_hwnd` under a name that doesn't presume a handle.
pub fn note_window_handle(handle: isize) {
    crate::tray_bridge::store_hwnd(handle);
}

/// Make the popup the foreground window. Reads the stored handle itself so
/// `HWND` stays inside the Windows arm; `ctx` is unused because Win32 does
/// the whole job here (macOS needs a viewport command as well).
pub fn focus_popup(_ctx: &egui::Context) {
    if let Some(hwnd) = crate::tray_bridge::load_hwnd() {
        crate::tray_bridge::force_foreground(HWND(hwnd as *mut _));
    }
}

/// No-op: the tray thread already polls the deadline on its own message
/// pump and `ShowWindow`s the popup when it expires, so no tick here ever
/// claims a reveal.
pub fn tick(_ctx: &egui::Context) -> bool {
    false
}

/// No-op: the frosted glass behind the card (spec §2) is a macOS
/// `NSVisualEffectView`. The Windows counterpart would be a DWM backdrop, and
/// it is not implemented — the card there is a plain translucent fill, as it
/// has always been.
pub fn sync_card_backdrop(_ctx: &egui::Context, _frame: &eframe::Frame, _card: egui::Rect) {}

/// The monitor `choose_screen` picked for the last show: its `rcMonitor` and
/// `rcWork`, in physical pixels of the virtual screen.
static CHOSEN_SCREEN: Mutex<Option<(RECT, RECT)>> = Mutex::new(None);

/// Every monitor's `rcMonitor` and `rcWork`, the primary first — the order
/// `screen_pick::pick_screen` falls back on.
fn monitors() -> Vec<(RECT, RECT)> {
    unsafe extern "system" fn push(m: HMONITOR, _: HDC, _: *mut RECT, out: LPARAM) -> BOOL {
        // SAFETY: `out` is the `&mut Vec` `monitors` passed, alive for the call.
        let out = unsafe { &mut *(out.0 as *mut Vec<(RECT, RECT)>) };
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetMonitorInfoW(m, &mut info) }.as_bool() {
            let entry = (info.rcMonitor, info.rcWork);
            if info.dwFlags & MONITORINFOF_PRIMARY != 0 {
                out.insert(0, entry);
            } else {
                out.push(entry);
            }
        }
        TRUE
    }
    let mut out: Vec<(RECT, RECT)> = Vec::new();
    // SAFETY: the callback only runs inside this call, while `out` is alive.
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(push), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

fn px_rect(r: RECT) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(r.left as f32, r.top as f32),
        egui::pos2(r.right as f32, r.bottom as f32),
    )
}

/// Pick the popup's monitor (spec §1) and keep it for `screen_geometry`.
/// Called at show time, before the popup takes the foreground, so the
/// foreground window is still the one the user was in. With the popup
/// itself in front — a re-show while visible — the last choice stands.
pub fn choose_screen(popup: HWND) {
    let screens = monitors();
    if screens.is_empty() {
        return;
    }
    // SAFETY: plain Win32 reads into locals.
    let (fg, pointer) = unsafe {
        let mut p = POINT::default();
        let _ = GetCursorPos(&mut p);
        (GetForegroundWindow(), egui::pos2(p.x as f32, p.y as f32))
    };
    let mut chosen = CHOSEN_SCREEN.lock().unwrap();
    if fg == popup && chosen.is_some() {
        return;
    }
    let front = (!fg.0.is_null() && fg != popup)
        .then(|| {
            let mut r = RECT::default();
            // SAFETY: `fg` came from the OS just now; a stale handle fails the call.
            unsafe { GetWindowRect(fg, &mut r) }
                .ok()
                .map(|_| px_rect(r))
        })
        .flatten();
    let rects: Vec<egui::Rect> = screens.iter().map(|(m, _)| px_rect(*m)).collect();
    let i = crate::screen_pick::pick_screen(front, pointer, &rects);
    *chosen = screens.get(i).copied();
}

/// The monitor `choose_screen` picked, in points; the primary until the first
/// show. Pixels over the window's `pixels_per_point`, because that is what
/// egui multiplies a viewport command's points by on the way back out.
// TODO: a monitor with a different scale factor from the one the window is
// on — the conversion uses the window's current factor, not the target's.
pub fn screen_geometry(ctx: &egui::Context, _frame: &eframe::Frame) -> Option<ScreenGeometry> {
    let (monitor, work) = {
        let mut chosen = CHOSEN_SCREEN.lock().unwrap();
        if chosen.is_none() {
            *chosen = monitors().first().copied();
        }
        (*chosen)?
    };
    let ppp = ctx.pixels_per_point();
    let points = |r: RECT| {
        let r = px_rect(r);
        egui::Rect::from_min_max(
            (r.min.to_vec2() / ppp).to_pos2(),
            (r.max.to_vec2() / ppp).to_pos2(),
        )
    };
    Some(ScreenGeometry {
        monitor: points(monitor),
        work: points(work),
    })
}

/// No-op on Windows: the shortcut vocabulary here is words (`Ctrl+Shift+A`),
/// and every glyph in that is in egui's default fonts.
pub fn install_native_fonts(_ctx: &egui::Context) {}

/// No key source on Windows yet, so the hotkey row stays the typed field it
/// has always been — which is the point of returning `None` rather than
/// wiring a half-measure: **nothing on this platform has been measured**
/// (`docs/TASK-hotkey-native.md`), and a recorder built against documentation
/// is what that task exists to stop. The order of work there puts Windows
/// after its facts.
pub fn new_edge_source(_ctx: &egui::Context) -> Option<Box<dyn crate::recorder::EdgeSource>> {
    None
}

/// No conflict report on Windows. `RegisterHotKey` is *documented* to
/// arbitrate across processes and to return an already-registered error — but
/// documented is not measured, and this is the platform the task doc forbids
/// relying on documentation for. Saying nothing is the honest answer until
/// somebody runs the probe here.
pub fn hotkey_conflict(_hk: &Hotkey) -> Option<String> {
    None
}
