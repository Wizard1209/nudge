use std::path::PathBuf;
use std::sync::atomic::{AtomicIsize, AtomicU32, Ordering};
use std::thread;
use std::time::Instant;

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
use crate::config_watcher::{LiveChange, LiveConfig};
use crate::daisy;
use crate::hotkey::{self, Hotkey, Modifiers};
use crate::nudge_state;
use crate::platform::shared;

static HWND_STORE: AtomicIsize = AtomicIsize::new(0);

static TRAY_THREAD_ID: AtomicU32 = AtomicU32::new(0);

const WM_TRAY_WAKE: u32 = WM_USER + 1;

pub const WM_TRAY_QUIT: u32 = WM_USER + 2;

pub fn store_hwnd(hwnd: isize) {
    HWND_STORE.store(hwnd, Ordering::SeqCst);
}

pub fn load_hwnd() -> Option<isize> {
    let val = HWND_STORE.load(Ordering::SeqCst);
    if val != 0 { Some(val) } else { None }
}

// The system restricts which processes may set the foreground window, and SetForegroundWindow
// cannot force a window forward while the user is working with another window.
// proof: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setforegroundwindow
pub fn force_foreground(hwnd: HWND) {
    crate::platform::choose_screen(hwnd);
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

pub fn request_manual_open() {
    shared::set_popup_visible(true);
    shared::raise_tray_clicked();
    wake_eframe();
}

pub(crate) fn wake_tray_thread() {
    let tid = TRAY_THREAD_ID.load(Ordering::SeqCst);
    if tid != 0 {
        unsafe {
            let _ = PostThreadMessageW(tid, WM_TRAY_WAKE, WPARAM(0), LPARAM(0));
        }
    }
}

// RegisterHotKey with a null HWND posts WM_HOTKEY to the calling thread's message queue,
// and the message must be processed in that thread's message loop.
// proof: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-registerhotkey
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

const HOTKEY_ID_SHOW_POPUP: i32 = 1;

fn vk_for_key(key: &str) -> Option<u32> {
    if key.len() == 1 {
        let ch = key.as_bytes()[0];
        if ch.is_ascii_uppercase() || ch.is_ascii_digit() {
            return Some(ch as u32);
        }
    }
    if let Some(n) = hotkey::function_key_number(key) {
        return Some(0x70 + (n as u32) - 1);
    }
    match key {
        "SPACE" => Some(0x20),
        "ENTER" => Some(0x0D),
        "TAB" => Some(0x09),
        "ESCAPE" => Some(0x1B),
        "BACKSPACE" => Some(0x08),
        "LEFT" => Some(0x25),
        "UP" => Some(0x26),
        "RIGHT" => Some(0x27),
        "DOWN" => Some(0x28),
        "PAGEUP" => Some(0x21),
        "PAGEDOWN" => Some(0x22),
        "END" => Some(0x23),
        "HOME" => Some(0x24),
        "INSERT" => Some(0x2D),
        "DELETE" => Some(0x2E),
        "-" => Some(0xBD),
        "=" => Some(0xBB),
        "[" => Some(0xDB),
        "]" => Some(0xDD),
        "\\" => Some(0xDC),
        ";" => Some(0xBA),
        "'" => Some(0xDE),
        "," => Some(0xBC),
        "." => Some(0xBE),
        "/" => Some(0xBF),
        "`" => Some(0xC0),
        _ => None,
    }
}

fn modifiers_to_win32(m: Modifiers) -> HOT_KEY_MODIFIERS {
    let mut out = MOD_NOREPEAT;
    if m.contains(Modifiers::CTRL) {
        out |= MOD_CONTROL;
    }
    if m.contains(Modifiers::ALT) {
        out |= MOD_ALT;
    }
    if m.contains(Modifiers::SHIFT) {
        out |= MOD_SHIFT;
    }
    if m.contains(Modifiers::WIN) {
        out |= MOD_WIN;
    }
    out
}

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

fn tray_thread_main(hotkey: Option<Hotkey>, config_path: PathBuf, initial_config: config::Config) {
    unsafe {
        TRAY_THREAD_ID.store(GetCurrentThreadId(), Ordering::SeqCst);
    }

    if let Some(hk) = &hotkey {
        try_register_hotkey(hk);
    }

    let mut live_config = LiveConfig::new(config_path, initial_config);
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

    let initial_icon =
        tray_icon_frame(daisy::PETAL_COUNT, None).expect("failed to create initial tray icon");
    let initial_tooltip = "Nudge";

    let tray = TrayIconBuilder::new()
        .with_menu(Box::new(menu))
        .with_menu_on_left_click(false)
        .with_tooltip(initial_tooltip)
        .with_icon(initial_icon)
        .build()
        .expect("failed to build tray icon");

    tray_icon::TrayIconEvent::set_event_handler(Some(|event| {
        if matches!(
            event,
            tray_icon::TrayIconEvent::Click {
                button: tray_icon::MouseButton::Left,
                button_state: tray_icon::MouseButtonState::Up,
                ..
            }
        ) {
            request_manual_open();
        }
    }));

    tray_icon::menu::MenuEvent::set_event_handler(Some(
        move |event: tray_icon::menu::MenuEvent| {
            if event.id == quit_id {
                std::process::exit(0);
            }
            if event.id == show_id {
                request_manual_open();
            }
            if event.id == settings_id {
                drop(shared::spawn_settings_process(&settings_args()));
            }
        },
    ));

    let _menu_items = (show_item, settings_item, quit_item);

    let mut last_icon_key: Option<shared::IconKey> = None;
    let mut last_tooltip_minutes: Option<u64> = None;
    loop {
        drain_messages();

        if shared::take_config_reload_requested() {
            apply_config_reload(&mut live_config, &mut hotkey_registered);
        }

        if shared::popup_visible() {
            wait_for_message(u32::MAX);
            continue;
        }

        let timer = match shared::armed_timer() {
            Some(t) => t,
            None => {
                wait_for_message(u32::MAX);
                continue;
            }
        };

        let countdown = timer.observe(Instant::now());

        let icon_key = countdown.icon_key();
        if last_icon_key != Some(icon_key) {
            let frame = &countdown.frame;
            if let Ok(icon) = tray_icon_frame(frame.petals_remaining, frame.drift) {
                let _ = tray.set_icon(Some(icon));
            }
            last_icon_key = Some(icon_key);
        }

        let minutes = countdown.tooltip_minutes();
        if last_tooltip_minutes != Some(minutes) {
            let _ = tray.set_tooltip(Some(&countdown.tooltip()));
            last_tooltip_minutes = Some(minutes);
        }

        if countdown.frame.should_fire {
            shared::disarm_timer(timer);
            shared::raise_timer_fired();
            if let Some(hwnd_val) = load_hwnd() {
                force_foreground(HWND(hwnd_val as *mut _));
            }
            wait_for_message(u32::MAX);
            continue;
        }

        wait_for_message(countdown.next_wake().as_millis() as u32);
    }
}

fn apply_config_reload(live_config: &mut LiveConfig, hotkey_registered: &mut bool) {
    for change in live_config.reload() {
        match change {
            LiveChange::Hotkey(hk) => {
                if *hotkey_registered {
                    unsafe {
                        let _ = UnregisterHotKey(None, HOTKEY_ID_SHOW_POPUP);
                    }
                    *hotkey_registered = false;
                }
                *hotkey_registered = hk.is_some_and(|hk| try_register_hotkey(&hk));
            }
            LiveChange::Sprite(to) => {
                shared::park_sprite(to);
            }
        }
    }
}

fn tray_icon_frame(
    petals_remaining: u8,
    drift: Option<daisy::DriftState>,
) -> Result<tray_icon::Icon, tray_icon::BadIcon> {
    let size = tray_icon_pixels();
    let rgba = daisy::render(size, petals_remaining, drift);
    tray_icon::Icon::from_rgba(rgba.to_vec(), size, size)
}

// GetDpiForSystem returns 96 while the calling thread is DPI-unaware and the actual system
// DPI otherwise, so the system DPI is not to be cached.
// proof: https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-getdpiforsystem
fn tray_icon_pixels() -> u32 {
    use windows::Win32::UI::HiDpi::{GetDpiForSystem, GetSystemMetricsForDpi};
    use windows::Win32::UI::WindowsAndMessaging::SM_CXSMICON;
    let px = unsafe { GetSystemMetricsForDpi(SM_CXSMICON, GetDpiForSystem()) };
    if px > 0 { px as u32 } else { 16 }
}

fn wake_eframe() {
    if let Some(hwnd_val) = load_hwnd() {
        force_foreground(HWND(hwnd_val as *mut _));
    }
}

fn settings_args() -> Vec<String> {
    let tid = TRAY_THREAD_ID.load(Ordering::SeqCst);
    vec![
        crate::settings_app::TRAY_THREAD_ARG.to_string(),
        tid.to_string(),
    ]
}

fn drain_messages() {
    unsafe {
        let mut msg = MSG::default();
        while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
            if msg.message == WM_TRAY_QUIT {
                std::process::exit(0);
            }
            if msg.message == WM_HOTKEY && msg.wParam.0 == HOTKEY_ID_SHOW_POPUP as usize {
                request_manual_open();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_the_parser_accepts_has_a_vk() {
        let tokens = hotkey::all_key_tokens();
        assert!(tokens.len() > 80, "the grammar enumerates its keys");
        for token in tokens {
            assert!(vk_for_key(&token).is_some(), "{token} parses but has no VK");
        }
    }

    #[test]
    fn every_vk_is_distinct() {
        let mut seen = std::collections::HashMap::new();
        for token in hotkey::all_key_tokens() {
            let vk = vk_for_key(&token).unwrap();
            if let Some(other) = seen.insert(vk, token.clone()) {
                panic!("{token} and {other} share VK {vk:#x}");
            }
        }
    }

    #[test]
    fn the_symbol_caps_are_the_oem_position_codes() {
        assert_eq!(vk_for_key("-"), Some(0xBD));
        assert_eq!(vk_for_key("="), Some(0xBB));
        assert_eq!(vk_for_key("/"), Some(0xBF));
        assert_eq!(vk_for_key("`"), Some(0xC0));
    }

    #[test]
    fn unknown_tokens_have_no_vk() {
        for token in ["", "+", "ARROWUP", "NUMPAD5", "F0", "F99", "AA", "мама"] {
            assert!(vk_for_key(token).is_none(), "{token:?} must not map");
        }
    }
}
