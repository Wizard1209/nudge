#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(target_arch = "wasm32"))]
use nudge_web::{app, config, layout, platform, settings_app};
#[cfg(any(target_os = "windows", target_os = "macos"))]
use nudge_web::{autostart, config_watcher};

#[cfg(any(target_os = "windows", target_os = "macos"))]
fn run_autostart_selftest() -> ! {
    macro_rules! fail {
        ($($a:tt)*) => {{ eprintln!("[selftest] FAIL: {}", format!($($a)*)); std::process::exit(1); }};
    }

    let provider = match autostart::provider_for_current_exe(false) {
        Ok(p) => p,
        Err(e) => fail!("{e}"),
    };
    println!("[selftest] {}", provider.location());

    let initial = match provider.is_enabled() {
        Ok(v) => v,
        Err(e) => fail!("is_enabled (initial): {e}"),
    };
    println!("[selftest] initial is_enabled = {initial}");

    if let Err(e) = provider.enable() {
        fail!("enable: {e}");
    }
    let after_enable = provider.is_enabled().unwrap_or(false);
    println!("[selftest] after enable  -> is_enabled = {after_enable} (want true)");

    if let Err(e) = provider.enable() {
        fail!("enable (second time, must be idempotent): {e}");
    }

    if let Err(e) = provider.disable() {
        fail!("disable: {e}");
    }
    let after_disable = provider.is_enabled().unwrap_or(true);
    println!("[selftest] after disable -> is_enabled = {after_disable} (want false)");

    let restore = if initial {
        provider.enable()
    } else {
        provider.disable()
    };
    if let Err(e) = restore {
        fail!("restore to {initial}: {e}");
    }
    println!("[selftest] restored is_enabled = {initial}");

    if after_enable && !after_disable {
        println!("[selftest] PASS");
        std::process::exit(0);
    }
    fail!("enable/disable cycle did not flip the state as expected");
}

#[cfg(not(target_arch = "wasm32"))]
fn run_settings_ui() -> eframe::Result {
    use eframe::egui;
    use settings_app::SettingsApp;

    let Some(_settings_lock) = platform::acquire_settings_singleton() else {
        return Ok(());
    };

    let config_file = config::ConfigFile::from_args(std::env::args().skip(1));
    let (cfg, err) = config_file.load();
    if let Some(e) = err {
        eprintln!("[nudge-settings] {e}");
    }

    let tray_thread = settings_app::parse_tray_thread_arg(std::env::args().skip(1));
    let host = match settings_app::SettingsHost::native(
        config_file.path().to_path_buf(),
        cfg.autostart,
        tray_thread,
    ) {
        Ok(h) => h,
        Err(e) => {
            eprintln!("[nudge-settings] cannot construct autostart provider: {e}");
            return Err(eframe::Error::AppCreation(
                format!("cannot construct autostart provider: {e}").into(),
            ));
        }
    };

    let viewport = egui::ViewportBuilder::default()
        .with_inner_size(settings_app::WINDOW_SIZE)
        .with_decorations(true)
        .with_resizable(false)
        .with_title("Nudge — Settings");
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Nudge — Settings",
        options,
        Box::new(move |cc| Ok(Box::new(SettingsApp::new(cc, cfg, host)))),
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    use app::NudgeApp;
    use eframe::egui;

    platform::allow_evicted_file_downloads();

    if std::env::args()
        .skip(1)
        .any(|a| a == "--autostart-selftest")
    {
        #[cfg(any(target_os = "windows", target_os = "macos"))]
        run_autostart_selftest();
        #[cfg(not(any(target_os = "windows", target_os = "macos")))]
        {
            eprintln!("[nudge] --autostart-selftest is only supported on Windows and macOS");
            std::process::exit(2);
        }
    }

    if settings_app::parse_settings_arg(std::env::args().skip(1)) {
        return run_settings_ui();
    }

    let config_file = config::ConfigFile::from_args(std::env::args().skip(1));
    let (config, config_err) = config_file.load();
    if let Some(err) = config_err {
        eprintln!("[nudge] {err}");
    } else if let Err(e) = config_file.seed_default_if_missing() {
        eprintln!("[nudge] failed to write default config: {e}");
    }
    let config_path = config_file.path().to_path_buf();
    let configured_hotkey = config.hotkey_or_unbound();
    let (default_minutes, interval_was_invalid) = config.resolved_interval_minutes();
    if interval_was_invalid {
        eprintln!(
            "[nudge] config default_interval_minutes {} is not {}, falling back to {}",
            config.default_interval_minutes,
            config::interval_rule_text(),
            config::DEFAULT_INTERVAL_MINUTES
        );
    }

    platform::start(configured_hotkey, config_path.clone(), config.clone());

    // Dropping notify's watcher stops its watches, so the handle stays bound until main returns.
    // proof: https://docs.rs/crate/notify/8.2.0/source/src/windows.rs
    #[cfg(any(target_os = "windows", target_os = "macos"))]
    let _watcher = match config_watcher::spawn(
        config_path.clone(),
        Box::new(platform::request_config_reload),
    ) {
        Ok(h) => Some(h),
        Err(e) => {
            eprintln!("[nudge] config live-reload disabled — watcher spawn failed: {e}");
            None
        }
    };

    let win_size = [layout::POPUP_SIZE.0 as f32, layout::POPUP_SIZE.1 as f32];
    #[cfg(target_os = "windows")]
    let viewport = {
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
        let (screen_w, screen_h) = unsafe {
            (
                GetSystemMetrics(SM_CXSCREEN).max(1) as u32,
                GetSystemMetrics(SM_CYSCREEN).max(1) as u32,
            )
        };
        let (x, y) = layout::window_position(
            (screen_w, screen_h),
            (win_size[0] as u32, win_size[1] as u32),
        );
        egui::ViewportBuilder::default()
            .with_inner_size(win_size)
            .with_position([x as f32, y as f32])
            .with_decorations(false)
            .with_always_on_top()
            .with_resizable(false)
            .with_transparent(true)
            .with_taskbar(false)
            .with_active(false)
            .with_title("Nudge")
    };
    #[cfg(target_os = "macos")]
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size(win_size)
        .with_decorations(false)
        .with_always_on_top()
        .with_resizable(false)
        .with_transparent(true)
        .with_title("Nudge");
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size(win_size)
        .with_decorations(false)
        .with_always_on_top()
        .with_resizable(false)
        .with_title("Nudge");

    #[cfg_attr(not(target_os = "macos"), expect(unused_mut))]
    let mut options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::{ActivationPolicy, EventLoopBuilderExtMacOS};
        options.event_loop_builder = Some(Box::new(|builder| {
            builder.with_activation_policy(ActivationPolicy::Accessory);
        }));
    }

    eframe::run_native(
        "Nudge",
        options,
        Box::new(move |cc| {
            platform::start_on_event_loop(&cc.egui_ctx);
            Ok(Box::new(
                NudgeApp::new(cc, default_minutes).with_sprite_config(&config.sprite),
            ))
        }),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {}
