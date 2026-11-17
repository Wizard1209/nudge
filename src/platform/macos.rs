// egui resets each viewport's repaint delay when a pass begins (to Duration::MAX unless a
// repaint is outstanding), so request_repaint_after holds only if every pass re-issues it.
// proof: https://docs.rs/crate/egui/0.34.2/source/src/context.rs

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use eframe::egui;
use muda::{Menu, MenuEvent, MenuId, MenuItem};
use objc2::ClassType;
use objc2_app_kit::{NSApplication, NSEvent, NSScreen, NSView};
use objc2_foundation::{MainThreadMarker, NSRect};
use raw_window_handle::{HasWindowHandle, RawWindowHandle};

use super::shared::{self, Countdown, IconKey};
use crate::config;
use crate::config_watcher::{LiveChange, LiveConfig};
use crate::hotkey::{self, Hotkey};
use crate::layout::ScreenGeometry;
use crate::nudge_state;
use crate::screen_pick;

mod activation;
mod backdrop;
mod dataless;
pub mod front_window;
pub mod hotkey_carbon;
pub mod key_monitor;
pub mod keycode;
pub mod settings_lock;
pub mod status_item;
pub mod symbolic;
use activation::{hand_back_activation, remember_front_application};
use backdrop::hide_card_backdrop;
pub use backdrop::sync_card_backdrop;
pub use dataless::allow_evicted_file_downloads;
pub use settings_lock::acquire_settings_singleton;
use status_item::StatusItem;

static MANUAL_OPEN_REQUESTED: AtomicBool = AtomicBool::new(false);

static SETTINGS_REQUESTED: AtomicBool = AtomicBool::new(false);
static QUIT_REQUESTED: AtomicBool = AtomicBool::new(false);

static CHOSEN_SCREEN: Mutex<Option<NSRect>> = Mutex::new(None);

static PENDING_START: Mutex<Option<(Option<Hotkey>, PathBuf, config::Config)>> = Mutex::new(None);

static WAKE_CTX: Mutex<Option<egui::Context>> = Mutex::new(None);

thread_local! {
    static UI: RefCell<Option<UiState>> = const { RefCell::new(None) };
}

struct UiState {
    tray: Option<StatusItem>,
    // muda's NSMenuItem holds only a raw pointer to the muda item it fires, so the muda Menu
    // and MenuItem values are kept for as long as the menu can fire.
    // proof: https://docs.rs/crate/muda/0.15.3/source/src/platform_impl/macos/mod.rs
    _menu: Menu,
    _menu_items: [MenuItem; 3],
    last_icon_key: Option<IconKey>,
    last_tooltip_minutes: Option<u64>,
    reload: ReloadState,
}

struct ReloadState {
    registered: Option<hotkey_carbon::Registration>,
    config: LiveConfig,
}

pub fn start(hotkey: Option<Hotkey>, config_path: PathBuf, initial: config::Config) {
    shared::record_config_path(config_path.clone());
    *PENDING_START.lock().unwrap() = Some((hotkey, config_path, initial));
}

pub fn start_on_event_loop(ctx: &egui::Context) {
    *WAKE_CTX.lock().unwrap() = Some(ctx.clone());

    if let Some(mtm) = MainThreadMarker::new() {
        choose_screen(mtm);
    }

    let Some((hotkey, config_path, initial)) = PENDING_START.lock().unwrap().take() else {
        return;
    };

    let (menu, menu_items) = build_menu();
    install_event_handlers(menu_items.each_ref().map(|item| item.id().clone()));

    let tray = match MainThreadMarker::new()
        .ok_or_else(|| "not on the main thread".to_string())
        .and_then(|mtm| StatusItem::new(mtm, &menu))
    {
        Ok(t) => Some(t),
        Err(e) => {
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
                config: LiveConfig::new(config_path, initial),
            },
        });
    });
}

fn build_menu() -> (Menu, [MenuItem; 3]) {
    let menu = Menu::new();
    let items = nudge_state::TRAY_MENU_LABELS.map(|label| MenuItem::new(label, true, None));
    for item in &items {
        if let Err(e) = menu.append(item) {
            eprintln!("[nudge] could not append tray menu item: {e}");
        }
    }
    (menu, items)
}

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
}

pub fn request_manual_open() {
    MANUAL_OPEN_REQUESTED.store(true, Ordering::SeqCst);
    wake();
}

pub(super) fn wake() {
    if let Some(ctx) = WAKE_CTX.lock().unwrap().as_ref() {
        ctx.request_repaint();
    }
}

pub fn adopt_window(_frame: &eframe::Frame) {}

pub fn popup_is_foreground(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().focused) == Some(true)
}

pub fn set_popup_visible(visible: bool) {
    shared::set_popup_visible(visible);
    if !visible {
        hide_card_backdrop();
        hand_back_activation();
    }
}

#[must_use = "a claimed reveal is only ordered in by the caller"]
pub fn tick(ctx: &egui::Context) -> bool {
    if QUIT_REQUESTED.swap(false, Ordering::SeqCst) {
        std::process::exit(0);
    }
    if SETTINGS_REQUESTED.swap(false, Ordering::SeqCst) {
        spawn_settings_process();
    }
    reap_settings_children();
    if shared::take_config_reload_requested() {
        apply_config_reload();
    }

    let popup_visible = shared::popup_visible();

    if MANUAL_OPEN_REQUESTED.swap(false, Ordering::SeqCst) && !popup_visible {
        shared::raise_tray_clicked();
        reveal(ctx);
        return true;
    }

    if popup_visible {
        return false;
    }

    let Some(timer) = shared::armed_timer() else {
        return false;
    };

    let countdown = timer.observe(Instant::now());

    refresh_status_item(&countdown);

    if !countdown.frame.should_fire {
        ctx.request_repaint_after(countdown.next_wake());
        return false;
    }

    shared::disarm_timer(timer);
    shared::raise_timer_fired();
    reveal(ctx);
    true
}

fn reveal(ctx: &egui::Context) {
    if let Some(mtm) = MainThreadMarker::new() {
        choose_screen(mtm);
    }
    set_popup_visible(true);
    ctx.request_repaint();
}

// kCGWindowBounds is in screen space with its origin at the upper-left corner of the main
// display.
// proof: https://developer.apple.com/documentation/coregraphics/kcgwindowbounds
fn choose_screen(mtm: MainThreadMarker) {
    let screens = NSScreen::screens(mtm);
    let Some(primary) = (unsafe { screens.firstObject() }) else {
        return;
    };
    let primary_height = primary.frame().size.height;
    let frames: Vec<NSRect> = screens.iter().map(|s| s.frame()).collect();
    let rects: Vec<egui::Rect> = frames
        .iter()
        .map(|&r| top_left_rect(r, primary_height, 1.0))
        .collect();
    let front = front_window::front_window_bounds().map(|r| {
        egui::Rect::from_min_size(
            egui::pos2(r.origin.x as f32, r.origin.y as f32),
            egui::vec2(r.size.width as f32, r.size.height as f32),
        )
    });
    let mouse = unsafe { NSEvent::mouseLocation() };
    let pointer = egui::pos2(mouse.x as f32, (primary_height - mouse.y) as f32);
    let chosen = screen_pick::pick_screen(front, pointer, &rects);
    *CHOSEN_SCREEN.lock().unwrap() = frames.get(chosen).copied();
}

// winit's macOS focus_window activates the app and keys the window only while the window is
// visible and not miniaturized; otherwise it does nothing.
// proof: https://docs.rs/crate/winit/0.30.13/source/src/platform_impl/macos/window_delegate.rs
pub fn focus_popup(ctx: &egui::Context) {
    if let Some(mtm) = MainThreadMarker::new() {
        remember_front_application();
        let app = NSApplication::sharedApplication(mtm);
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

pub fn screen_geometry(ctx: &egui::Context, frame: &eframe::Frame) -> Option<ScreenGeometry> {
    let mtm = MainThreadMarker::new()?;
    let Ok(handle) = frame.window_handle() else {
        return None;
    };
    let RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
        return None;
    };
    let content: &NSView = unsafe { &*appkit.ns_view.as_ptr().cast::<NSView>() };
    let screens = NSScreen::screens(mtm);
    let chosen = *CHOSEN_SCREEN.lock().unwrap();
    let screen = chosen
        .and_then(|frame| screens.iter().find(|s| s.frame() == frame))
        .map(|s| s.retain())
        .or_else(|| content.window().and_then(|w| w.screen()))
        .or_else(|| NSScreen::mainScreen(mtm))?;
    let primary = unsafe { screens.firstObject() }?;
    let primary_height = primary.frame().size.height;
    let zoom = f64::from(ctx.zoom_factor());
    Some(ScreenGeometry {
        monitor: top_left_rect(screen.frame(), primary_height, zoom),
        work: top_left_rect(screen.visibleFrame(), primary_height, zoom),
    })
}

fn top_left_rect(r: NSRect, primary_height: f64, zoom: f64) -> egui::Rect {
    let top = primary_height - (r.origin.y + r.size.height);
    egui::Rect::from_min_size(
        egui::pos2((r.origin.x / zoom) as f32, (top / zoom) as f32),
        egui::vec2((r.size.width / zoom) as f32, (r.size.height / zoom) as f32),
    )
}

fn refresh_status_item(countdown: &Countdown) {
    let minutes = countdown.tooltip_minutes();
    let icon_key = countdown.icon_key();

    UI.with_borrow_mut(|slot| {
        let Some(ui) = slot.as_mut() else {
            return;
        };
        let UiState {
            tray,
            last_icon_key,
            last_tooltip_minutes,
            ..
        } = ui;
        let Some(tray) = tray.as_ref() else {
            return;
        };

        if *last_icon_key != Some(icon_key) {
            tray.set_frame(countdown.frame.petals_remaining, countdown.frame.drift);
            *last_icon_key = Some(icon_key);
        }

        if *last_tooltip_minutes != Some(minutes) {
            tray.set_tooltip(&countdown.tooltip());
            *last_tooltip_minutes = Some(minutes);
        }
    });
}

static NUDGE_HOTKEY: Mutex<Option<Hotkey>> = Mutex::new(None);

pub fn note_own_hotkey(hk: Option<Hotkey>) {
    *NUDGE_HOTKEY.lock().unwrap() = hk;
}

fn holds_this_hotkey(hk: &Hotkey) -> bool {
    NUDGE_HOTKEY.lock().unwrap().as_ref() == Some(hk)
}

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
            eprintln!(
                "[nudge] registering hotkey \"{label}\" failed: {e} — running with no hotkey"
            );
            note_own_hotkey(None);
            None
        }
    }
}

static SETTINGS_CHILDREN: Mutex<Vec<std::process::Child>> = Mutex::new(Vec::new());

fn spawn_settings_process() {
    if let Some(child) = shared::spawn_settings_process(&[]) {
        SETTINGS_CHILDREN.lock().unwrap().push(child);
    }
}

fn reap_settings_children() {
    let mut children = SETTINGS_CHILDREN.lock().unwrap();
    if children.is_empty() {
        return;
    }
    reap_exited(&mut children);
}

fn reap_exited(children: &mut Vec<std::process::Child>) {
    children.retain_mut(|child| match child.try_wait() {
        Ok(Some(_)) => false,
        Ok(None) => true,
        Err(e) => {
            eprintln!("[nudge] could not wait on the settings process: {e}");
            false
        }
    });
}

fn apply_config_reload() {
    UI.with_borrow_mut(|slot| {
        let Some(ui) = slot.as_mut() else {
            return;
        };
        reload_config(&mut ui.reload);
    });
}

fn reload_config(state: &mut ReloadState) {
    for change in state.config.reload() {
        match change {
            LiveChange::Hotkey(hk) => {
                state.registered = None;
                note_own_hotkey(None);
                state.registered = hk.and_then(|hk| try_register_hotkey(&hk));
            }
            LiveChange::Sprite(to) => {
                shared::park_sprite(to);
                wake();
            }
        }
    }
}

const SYMBOL_FONT_PATH: &str = "/System/Library/Fonts/Apple Symbols.ttf";
const SYMBOL_FONT_NAME: &str = "apple-symbols";

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

pub fn new_edge_source(ctx: &egui::Context) -> Option<Box<dyn crate::recorder::EdgeSource>> {
    key_monitor::KeyMonitor::install(ctx.clone())
        .map(|m| Box::new(m) as Box<dyn crate::recorder::EdgeSource>)
}

pub fn hotkey_conflict(hk: &Hotkey) -> Option<String> {
    match hotkey_carbon::assess(hk) {
        hotkey_carbon::HotkeyStatus::Free | hotkey_carbon::HotkeyStatus::HeldByNudge => None,
        status => Some(status.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use objc2_foundation::{NSPoint, NSSize};

    use super::shared::{
        ArmedTimer, set_timer_state, take_pending_sprite, take_timer_fired, take_tray_clicked,
    };
    use super::*;

    #[test]
    fn the_process_can_be_told_which_hotkey_is_its_own() {
        let hk = hotkey::parse("Alt+Cmd+J").unwrap();
        note_own_hotkey(Some(hk.clone()));
        assert!(holds_this_hotkey(&hk));
        assert!(!holds_this_hotkey(&hotkey::parse("Alt+Cmd+K").unwrap()));
        note_own_hotkey(None);
        assert!(!holds_this_hotkey(&hk));
    }

    const INTERVAL: Duration = Duration::from_secs(600);

    fn ns_rect(x: f64, y: f64, w: f64, h: f64) -> NSRect {
        NSRect::new(NSPoint::new(x, y), NSSize::new(w, h))
    }

    #[test]
    fn the_primary_screen_flips_onto_the_origin() {
        let primary = ns_rect(0.0, 0.0, 1512.0, 982.0);
        assert_eq!(
            top_left_rect(primary, 982.0, 1.0),
            egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1512.0, 982.0))
        );
    }

    #[test]
    fn a_screen_above_the_primary_has_a_negative_top() {
        let above = ns_rect(0.0, 982.0, 1920.0, 1080.0);
        assert_eq!(top_left_rect(above, 982.0, 1.0).top(), -1080.0);
        let work = ns_rect(0.0, 0.0, 1512.0, 957.0);
        assert_eq!(top_left_rect(work, 982.0, 1.0).top(), 25.0);
    }

    #[test]
    fn zoom_divides_every_coordinate() {
        let r = ns_rect(100.0, 0.0, 800.0, 600.0);
        assert_eq!(
            top_left_rect(r, 1000.0, 2.0),
            egui::Rect::from_min_size(egui::pos2(50.0, 200.0), egui::vec2(400.0, 300.0))
        );
    }

    static TICK_GLOBALS: Mutex<()> = Mutex::new(());

    fn tick_globals() -> std::sync::MutexGuard<'static, ()> {
        TICK_GLOBALS.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn reset_globals() {
        let _ = shared::take_tray_clicked();
        let _ = shared::take_timer_fired();
        let _ = MANUAL_OPEN_REQUESTED.swap(false, Ordering::SeqCst);
        shared::clear_timer();
        set_popup_visible(false);
    }

    #[test]
    fn tick_turns_a_handler_flag_into_the_edge_the_app_reads() {
        let _guard = tick_globals();
        reset_globals();
        let ctx = egui::Context::default();

        set_popup_visible(false);
        request_manual_open();
        assert!(tick(&ctx), "a manual open while hidden claims a reveal");
        assert!(
            take_tray_clicked(),
            "a manual trigger raised while the card is hidden must reach the app"
        );

        set_popup_visible(true);
        request_manual_open();
        assert!(!tick(&ctx), "nothing to reveal while the card is up");
        assert!(
            !take_tray_clicked(),
            "a trigger arriving while the card is already up must not queue up behind it"
        );
    }

    #[test]
    fn an_expiry_inside_a_manual_reveal_is_not_swallowed() {
        let _guard = tick_globals();
        reset_globals();
        let ctx = egui::Context::default();

        let interval = Duration::from_secs(600);
        set_timer_state(ArmedTimer {
            deadline: Instant::now() - Duration::from_secs(1),
            duration: interval,
        });
        request_manual_open();

        assert!(tick(&ctx), "the manual open claims a reveal");
        assert!(take_tray_clicked(), "the manual open reaches the app");
        assert!(
            !take_timer_fired(),
            "the manual branch returns before the expiry is even looked at"
        );

        assert!(!tick(&ctx), "a re-tick while visible claims nothing");
        assert!(!tick(&ctx), "a re-tick while visible claims nothing");
        assert!(
            !take_timer_fired(),
            "an expiry must not be raised while a reveal is still unacknowledged"
        );
        assert!(
            shared::armed_timer().is_some(),
            "and the deadline must still be armed — nothing else would re-arm it"
        );

        set_popup_visible(false);
        assert!(tick(&ctx), "the expiry claims a reveal");
        assert!(
            take_timer_fired(),
            "the overdue nudge fires once the manual card is out of the way"
        );
        assert!(
            shared::armed_timer().is_none(),
            "and one deadline still fires exactly once"
        );
    }

    #[test]
    fn a_reveal_claims_the_popup_as_visible() {
        let _guard = tick_globals();
        reset_globals();
        let ctx = egui::Context::default();

        reveal(&ctx);
        assert!(
            shared::popup_visible(),
            "a claimed reveal is the popup up; `tick` must park until `hide_popup` says otherwise"
        );
    }

    #[test]
    fn arming_a_deadline_wakes_the_event_loop() {
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
        assert!(
            settled,
            "the harness must start from a context with nothing pending, \
             or this test cannot see the wake it exists for"
        );

        set_timer_state(ArmedTimer {
            deadline: Instant::now() + Duration::from_secs(600),
            duration: INTERVAL,
        });
        assert!(
            ctx.has_requested_repaint(),
            "arming a deadline must ask for the frame that will schedule it"
        );

        *WAKE_CTX.lock().unwrap() = None;
    }

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
            config: LiveConfig::new(path.clone(), before),
        };
        let _guard = tick_globals();
        reload_config(&mut state);

        assert_eq!(
            take_pending_sprite(),
            Some(after.sprite.clone()),
            "the sprite block is parked for the app to apply while hidden (§10)"
        );

        assert_eq!(
            state.config.current(),
            &after,
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
            config: LiveConfig::new(path, before.clone()),
        };
        reload_config(&mut state);

        assert_eq!(state.config.current(), &before);
        assert_eq!(
            take_pending_sprite(),
            None,
            "nothing changed, nothing parked"
        );
    }

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
            config: LiveConfig::new(path, before),
        };
        reload_config(&mut state);

        assert!(
            ctx.has_requested_repaint(),
            "a parked sprite change must ask for the frame that applies it"
        );
        assert_eq!(take_pending_sprite(), Some(after.sprite));
        *WAKE_CTX.lock().unwrap() = None;
    }

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
