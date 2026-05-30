#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(target_arch = "wasm32"))]
mod app;
#[cfg(not(target_arch = "wasm32"))]
mod autostart;
#[cfg(not(target_arch = "wasm32"))]
mod config;
#[cfg(target_os = "windows")]
mod daisy;
mod hotkey;
#[cfg(not(target_arch = "wasm32"))]
mod journal;
#[cfg(not(target_arch = "wasm32"))]
mod nudge_state;
#[cfg(not(target_arch = "wasm32"))]
mod settings_app;
#[cfg(not(target_arch = "wasm32"))]
mod timer;
#[cfg(target_os = "windows")]
mod tray_bridge;
#[cfg(not(target_arch = "wasm32"))]
mod word_jump;

/// Exercise the real autostart provider against the live registry and report
/// PASS/FAIL, restoring whatever state we found. A manual QA tool — the
/// registry can't be unit-tested in CI/WSL — invoked via
/// `nudge --autostart-selftest`. Cross-check independently with
/// `scripts/autostart-check.ps1`.
#[cfg(target_os = "windows")]
fn run_autostart_selftest() -> ! {
    use autostart::{AutostartProvider, RUN_SUBKEY, RUN_VALUE_NAME, WindowsRegistryProvider};

    macro_rules! fail {
        ($($a:tt)*) => {{ eprintln!("[selftest] FAIL: {}", format!($($a)*)); std::process::exit(1); }};
    }

    let provider = match WindowsRegistryProvider::for_current_exe() {
        Ok(p) => p,
        Err(e) => fail!("{e}"),
    };
    println!("[selftest] HKCU\\{RUN_SUBKEY} value \"{RUN_VALUE_NAME}\"");

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

    if let Err(e) = provider.disable() {
        fail!("disable: {e}");
    }
    let after_disable = provider.is_enabled().unwrap_or(true);
    println!("[selftest] after disable -> is_enabled = {after_disable} (want false)");

    // Leave the registry exactly as we found it.
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
    fail!("enable/disable cycle did not flip the value as expected");
}

/// Acquire a process-wide named mutex so only one settings window exists at
/// a time. The tray menu may be clicked multiple times in a row; without
/// this guard each click would spawn a fresh window. We don't release the
/// mutex explicitly — Windows reclaims it when the process exits.
///
/// Returns `Some(handle)` to be held for the process lifetime when we are
/// the first owner. Returns `None` if the mutex already exists — caller
/// should exit silently in that case.
#[cfg(target_os = "windows")]
fn acquire_settings_singleton() -> Option<windows::Win32::Foundation::HANDLE> {
    use windows::Win32::Foundation::{ERROR_ALREADY_EXISTS, GetLastError};
    use windows::Win32::System::Threading::CreateMutexW;
    use windows::core::PCWSTR;

    // Name lives in the kernel namespace global to the user session;
    // `Global\` would require admin rights, so use a per-session name and
    // accept that two users on the same machine each get one window.
    let name: Vec<u16> = "Local\\NudgeSettings\0".encode_utf16().collect();
    unsafe {
        let handle = match CreateMutexW(None, true, PCWSTR(name.as_ptr())) {
            Ok(h) => h,
            Err(_) => return Some(windows::Win32::Foundation::HANDLE::default()),
        };
        if GetLastError() == ERROR_ALREADY_EXISTS {
            // Another settings window owns the mutex — back out silently.
            return None;
        }
        Some(handle)
    }
}

/// Render the settings window. Loads the config from the same source as the
/// main app, constructs the autostart provider, and hands both to
/// `SettingsApp`. The window is opaque, has decorations + taskbar entry,
/// and lives in its own process. The two processes communicate only
/// through config.json (and the registry for autostart).
#[cfg(not(target_arch = "wasm32"))]
fn run_settings_ui() -> eframe::Result {
    use eframe::egui;
    use settings_app::SettingsApp;

    // Single-instance guard — Windows-only. On non-Windows we don't ship
    // the settings UI in practice (it's launched from the tray, which is
    // Windows-only), but the code still needs to compile for `cargo build`
    // on the linux dev host.
    #[cfg(target_os = "windows")]
    let _mutex_handle = match acquire_settings_singleton() {
        Some(h) => h,
        None => return Ok(()),
    };

    // Load config from the same path the main process uses, so a saved
    // change is picked up by the running app at its next launch. (Live
    // reload is Task 5.)
    let cli_config_path = config::parse_config_arg(std::env::args().skip(1));
    let config_path = cli_config_path
        .clone()
        .unwrap_or_else(config::resolve_default_config_path);
    let (cfg, err) = config::load_or_default(&config_path);
    if let Some(e) = err {
        eprintln!("[nudge-settings] {e}");
    }

    // Provider: real registry on Windows, an in-memory fake on non-Windows
    // so the linux dev build compiles. Non-Windows users don't actually
    // reach the settings window (no tray to spawn it), so the fake never
    // runs in production.
    //
    // On Windows we refuse to open the window with a fake substitute when
    // the real provider fails to construct (e.g. `current_exe()` returned
    // Err — astronomically rare). Falling back to a FakeProvider would make
    // the autostart toggle a silent lie: the in-memory state would flip,
    // the config would persist `autostart: true`, but the registry would
    // never be touched — breaking Task 2's transactional invariant.
    #[cfg(target_os = "windows")]
    let provider: Box<dyn autostart::AutostartProvider> = {
        match autostart::WindowsRegistryProvider::for_current_exe() {
            Ok(p) => Box::new(p),
            Err(e) => {
                eprintln!("[nudge-settings] cannot construct autostart provider: {e}");
                return Err(eframe::Error::AppCreation(
                    format!("cannot construct autostart provider: {e}").into(),
                ));
            }
        }
    };
    #[cfg(not(target_os = "windows"))]
    let provider: Box<dyn autostart::AutostartProvider> =
        Box::new(autostart::FakeProvider::new(cfg.autostart));

    // Persistence closure: write atomically to the same config path the
    // main process reads from on launch. Move-captures the path; the
    // closure is held by SettingsApp for the lifetime of the window.
    let path_for_persist = config_path.clone();
    let persist: settings_app::PersistFn = Box::new(move |c: &config::Config| {
        config::save(&path_for_persist, c)
    });

    let win_size = [520.0_f32, 400.0];
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size(win_size)
        .with_min_inner_size([400.0, 320.0])
        .with_decorations(true)
        .with_resizable(true)
        .with_title("Nudge — Настройки");
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Nudge — Настройки",
        options,
        Box::new(move |cc| Ok(Box::new(SettingsApp::new(cc, cfg, provider, persist)))),
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    use app::NudgeApp;
    use eframe::egui;

    // Manual registry QA hook — runs the autostart lifecycle and exits before
    // any GUI / config work. Windows-only; a no-op stub elsewhere.
    if std::env::args().skip(1).any(|a| a == "--autostart-selftest") {
        #[cfg(target_os = "windows")]
        run_autostart_selftest();
        #[cfg(not(target_os = "windows"))]
        {
            eprintln!("[nudge] --autostart-selftest is only supported on Windows");
            std::process::exit(2);
        }
    }

    // Settings UI dispatch — runs in its own process so the main popup,
    // tray, and hotkey are untouched. The tray menu spawns nudge.exe with
    // this flag; we exit when the settings window closes.
    if settings_app::parse_settings_arg(std::env::args().skip(1)) {
        return run_settings_ui();
    }

    // Load the user config (or use defaults). Bad / missing files are
    // logged but never fatal — the app must always come up.
    //
    // `--config <path>` overrides the default location. When the override is
    // set we also skip first-run seeding: a caller pointing at a custom file
    // (perf test, throwaway sandbox) doesn't want a default file silently
    // materialized alongside it.
    let cli_config_path = config::parse_config_arg(std::env::args().skip(1));
    let config_path = cli_config_path
        .clone()
        .unwrap_or_else(config::resolve_default_config_path);
    let (config, config_err) = config::load_or_default(&config_path);
    if let Some(err) = config_err {
        eprintln!("[nudge] {err}");
    } else if cli_config_path.is_none() && !config_path.exists() {
        // First run: seed the file so users have a template to edit.
        if let Err(e) = config::ensure_default_written(&config_path) {
            eprintln!("[nudge] failed to write default config: {e}");
        }
    }
    let (configured_hotkey, hotkey_was_invalid) = config.resolved_hotkey();
    if hotkey_was_invalid {
        eprintln!(
            "[nudge] config hotkey \"{}\" is unparseable, falling back to default",
            config.hotkey
        );
    }
    let (default_minutes, interval_was_invalid) = config.resolved_interval_minutes();
    if interval_was_invalid {
        eprintln!(
            "[nudge] config default_interval_minutes {} is not a positive finite number, falling back to 10",
            config.default_interval_minutes
        );
    }

    // The tray icon, its menu, and the animation loop all live on a single
    // dedicated thread (see tray_bridge::spawn_tray_thread). It runs its
    // own message pump so animation keeps ticking even while eframe's
    // popup window is SW_HIDE'd. The same thread also owns the global
    // hotkey registration (WM_HOTKEY is delivered to the registering
    // thread's message queue).
    #[cfg(target_os = "windows")]
    tray_bridge::spawn_tray_thread(Some(configured_hotkey));
    #[cfg(not(target_os = "windows"))]
    let _ = configured_hotkey; // unused on non-Windows targets for now

    // Spotlight window: horizontally centered, vertical center at 40% of screen
    // per spec §1. Computed once at launch from primary monitor dimensions.
    let win_size = [520.0_f32, 320.0];
    #[cfg(target_os = "windows")]
    let viewport = {
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
        let (screen_w, screen_h) = unsafe {
            (
                GetSystemMetrics(SM_CXSCREEN).max(1) as u32,
                GetSystemMetrics(SM_CYSCREEN).max(1) as u32,
            )
        };
        let (x, y) =
            nudge_state::window_position((screen_w, screen_h), (win_size[0] as u32, win_size[1] as u32));
        egui::ViewportBuilder::default()
            .with_inner_size(win_size)
            .with_position([x as f32, y as f32])
            .with_decorations(false)
            .with_always_on_top()
            .with_resizable(false)
            .with_transparent(true)
            .with_taskbar(false)
            .with_title("Nudge")
    };
    #[cfg(not(target_os = "windows"))]
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size(win_size)
        .with_decorations(false)
        .with_always_on_top()
        .with_resizable(false)
        .with_title("Nudge");

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Nudge",
        options,
        Box::new(move |cc| Ok(Box::new(NudgeApp::new(cc, default_minutes)))),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {}
