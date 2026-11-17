use std::sync::Mutex;
use std::sync::atomic::{AtomicIsize, Ordering};

use ::windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, POINT, RECT, TRUE, WPARAM};
use ::windows::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use ::windows::Win32::UI::Shell::{
    ABE_BOTTOM, ABE_LEFT, ABE_RIGHT, ABE_TOP, ABM_GETAUTOHIDEBAREX, APPBARDATA, SHAppBarMessage,
};
use ::windows::Win32::UI::WindowsAndMessaging::{
    CallWindowProcW, GWL_EXSTYLE, GWLP_WNDPROC, GetCursorPos, GetForegroundWindow,
    GetWindowLongPtrW, GetWindowRect, MONITORINFOF_PRIMARY, STYLESTRUCT, SW_HIDE,
    SetWindowLongPtrW, ShowWindow, WM_STYLECHANGING, WNDPROC, WS_EX_TOOLWINDOW,
};
use ::windows::core::BOOL;
use eframe::egui;

use crate::config;
use crate::hotkey::Hotkey;
use crate::layout::ScreenGeometry;

pub use crate::tray_bridge::request_manual_open;

pub(super) fn wake() {
    crate::tray_bridge::wake_tray_thread();
}

pub fn allow_evicted_file_downloads() {}

pub fn start(hotkey: Option<Hotkey>, config_path: std::path::PathBuf, initial: config::Config) {
    super::shared::record_config_path(config_path.clone());
    crate::tray_bridge::spawn_tray_thread(hotkey, config_path, initial);
    remove_replaced_exe();
}

fn remove_replaced_exe() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    std::thread::spawn(move || {
        for _ in 0..40 {
            match crate::update::remove_old(&exe) {
                Ok(_) => return,
                Err(_) => std::thread::sleep(std::time::Duration::from_millis(250)),
            }
        }
        eprintln!(
            "[nudge] could not remove {}",
            crate::update::old_path(&exe).display()
        );
    });
}

const QUIT_TIMEOUT_MS: u32 = 10_000;

pub fn hand_over(exe: &std::path::Path, tray_thread: Option<u32>) -> Result<(), String> {
    use ::windows::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use ::windows::Win32::System::Threading::{
        GetProcessIdOfThread, OpenProcess, OpenThread, PROCESS_SYNCHRONIZE,
        THREAD_QUERY_LIMITED_INFORMATION, WaitForSingleObject,
    };
    use ::windows::Win32::UI::WindowsAndMessaging::PostThreadMessageW;

    const NOT_QUIT: &str = "Nudge did not quit; restart it to finish";
    let tid = tray_thread.ok_or(NOT_QUIT)?;
    unsafe {
        let process = OpenThread(THREAD_QUERY_LIMITED_INFORMATION, false, tid)
            .ok()
            .and_then(|thread| {
                let pid = GetProcessIdOfThread(thread);
                let _ = CloseHandle(thread);
                (pid != 0 && pid != std::process::id())
                    .then(|| OpenProcess(PROCESS_SYNCHRONIZE, false, pid).ok())
                    .flatten()
            });
        if let Some(process) = process {
            let posted =
                PostThreadMessageW(tid, crate::tray_bridge::WM_TRAY_QUIT, WPARAM(0), LPARAM(0))
                    .is_ok();
            let ended = posted && WaitForSingleObject(process, QUIT_TIMEOUT_MS) == WAIT_OBJECT_0;
            let _ = CloseHandle(process);
            if !ended {
                return Err(NOT_QUIT.into());
            }
        }
    }
    // RegisterHotKey fails for keystrokes already registered for another hot key, so the old
    // process must have ended before the new exe can register the same combination.
    // proof: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerhotkey
    std::process::Command::new(exe)
        .spawn()
        .map(drop)
        .map_err(|e| format!("Could not start the new Nudge: {e}"))
}

pub fn start_on_event_loop(_ctx: &egui::Context) {}

pub fn adopt_window(frame: &eframe::Frame) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    if let Ok(wh) = frame.window_handle()
        && let RawWindowHandle::Win32(h) = wh.as_raw()
    {
        let handle = h.hwnd.get();
        crate::tray_bridge::store_hwnd(handle);
        keep_tool_window(handle);
    }
}

pub fn set_popup_visible(visible: bool) {
    if !visible && let Some(hwnd) = crate::tray_bridge::load_hwnd() {
        unsafe {
            let _ = ShowWindow(HWND(hwnd as *mut _), SW_HIDE);
        }
    }
    super::shared::set_popup_visible(visible);
}

pub fn popup_is_foreground(_ctx: &egui::Context) -> bool {
    let Some(hwnd) = crate::tray_bridge::load_hwnd() else {
        return false;
    };
    let fg = unsafe { GetForegroundWindow() };
    !fg.0.is_null() && fg.0 as isize == hwnd
}

static WINIT_WNDPROC: AtomicIsize = AtomicIsize::new(0);

// winit's set_visible rewrites GWL_EXSTYLE from its own window flags, which never include
// WS_EX_TOOLWINDOW, so every show and hide clears that bit.
// proof: https://docs.rs/crate/winit/0.30.13/source/src/platform_impl/windows/window_state.rs
fn keep_tool_window(handle: isize) {
    unsafe extern "system" fn proc(h: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
        if msg == WM_STYLECHANGING && w.0 as i32 == GWL_EXSTYLE.0 {
            let change = unsafe { &mut *(l.0 as *mut STYLESTRUCT) };
            change.styleNew |= WS_EX_TOOLWINDOW.0;
        }
        let winit: WNDPROC = unsafe { std::mem::transmute(WINIT_WNDPROC.load(Ordering::Relaxed)) };
        unsafe { CallWindowProcW(winit, h, msg, w, l) }
    }
    let hwnd = HWND(handle as *mut _);
    unsafe {
        let winit = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, proc as *const () as isize);
        WINIT_WNDPROC.store(winit, Ordering::Relaxed);
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_TOOLWINDOW.0 as isize);
    }
}

pub fn focus_popup(_ctx: &egui::Context) {
    if let Some(hwnd) = crate::tray_bridge::load_hwnd() {
        crate::tray_bridge::force_foreground(HWND(hwnd as *mut _));
    }
}

pub fn tick(_ctx: &egui::Context) -> bool {
    false
}

pub fn sync_card_backdrop(_ctx: &egui::Context, _frame: &eframe::Frame, _card: egui::Rect) {}

static CHOSEN_SCREEN: Mutex<Option<(RECT, RECT)>> = Mutex::new(None);

fn monitors() -> Vec<(RECT, RECT)> {
    unsafe extern "system" fn push(m: HMONITOR, _: HDC, _: *mut RECT, out: LPARAM) -> BOOL {
        let out = unsafe { &mut *(out.0 as *mut Vec<(RECT, RECT)>) };
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if unsafe { GetMonitorInfoW(m, &mut info) }.as_bool() {
            let entry = (info.rcMonitor, shield_area(info.rcMonitor, info.rcWork));
            if info.dwFlags & MONITORINFOF_PRIMARY != 0 {
                out.insert(0, entry);
            } else {
                out.push(entry);
            }
        }
        TRUE
    }
    let mut out: Vec<(RECT, RECT)> = Vec::new();
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(push), LPARAM(&mut out as *mut _ as isize));
    }
    out
}

const AUTOHIDE_STRIP_PX: i32 = 2;

fn shield_area(monitor: RECT, work: RECT) -> RECT {
    let mut area = work;
    for edge in [ABE_LEFT, ABE_TOP, ABE_RIGHT, ABE_BOTTOM] {
        let mut bar = APPBARDATA {
            cbSize: std::mem::size_of::<APPBARDATA>() as u32,
            uEdge: edge,
            rc: monitor,
            ..Default::default()
        };
        if unsafe { SHAppBarMessage(ABM_GETAUTOHIDEBAREX, &mut bar) } == 0 {
            continue;
        }
        match edge {
            ABE_LEFT => area.left = area.left.max(monitor.left + AUTOHIDE_STRIP_PX),
            ABE_TOP => area.top = area.top.max(monitor.top + AUTOHIDE_STRIP_PX),
            ABE_RIGHT => area.right = area.right.min(monitor.right - AUTOHIDE_STRIP_PX),
            _ => area.bottom = area.bottom.min(monitor.bottom - AUTOHIDE_STRIP_PX),
        }
    }
    area
}

fn px_rect(r: RECT) -> egui::Rect {
    egui::Rect::from_min_max(
        egui::pos2(r.left as f32, r.top as f32),
        egui::pos2(r.right as f32, r.bottom as f32),
    )
}

pub fn choose_screen(popup: HWND) {
    let screens = monitors();
    if screens.is_empty() {
        return;
    }
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
            unsafe { GetWindowRect(fg, &mut r) }
                .ok()
                .map(|_| px_rect(r))
        })
        .flatten();
    let rects: Vec<egui::Rect> = screens.iter().map(|(m, _)| px_rect(*m)).collect();
    let i = crate::screen_pick::pick_screen(front, pointer, &rects);
    *chosen = screens.get(i).copied();
}

pub fn screen_geometry(ctx: &egui::Context, frame: &eframe::Frame) -> Option<ScreenGeometry> {
    use raw_window_handle::HasWindowHandle;
    frame.window_handle().ok()?;
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

pub struct SettingsLock {
    _mutex: HANDLE,
}

pub fn acquire_settings_singleton() -> Option<SettingsLock> {
    use ::windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use ::windows::Win32::System::Threading::CreateMutexW;
    use ::windows::core::PCWSTR;

    let name: Vec<u16> = "Local\\NudgeSettings\0".encode_utf16().collect();
    unsafe {
        let mutex = match CreateMutexW(None, true, PCWSTR(name.as_ptr())) {
            Ok(h) => h,
            Err(_) => {
                return Some(SettingsLock {
                    _mutex: HANDLE::default(),
                });
            }
        };
        if GetLastError() == ERROR_ALREADY_EXISTS {
            return None;
        }
        Some(SettingsLock { _mutex: mutex })
    }
}

pub fn install_native_fonts(_ctx: &egui::Context) {}

pub fn new_edge_source(_ctx: &egui::Context) -> Option<Box<dyn crate::recorder::EdgeSource>> {
    None
}

pub fn hotkey_conflict(_hk: &Hotkey) -> Option<String> {
    None
}
