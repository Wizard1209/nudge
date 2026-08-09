#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(target_arch = "wasm32"))]
mod app;
#[cfg(not(target_arch = "wasm32"))]
mod autostart;
#[cfg(not(target_arch = "wasm32"))]
mod config;
#[cfg(not(target_arch = "wasm32"))]
mod config_watcher;
#[cfg(any(target_os = "windows", target_os = "macos"))]
mod daisy;
mod hotkey;
#[cfg(not(target_arch = "wasm32"))]
mod journal;
#[cfg(not(target_arch = "wasm32"))]
mod nudge_state;
#[cfg(not(target_arch = "wasm32"))]
mod platform;
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

/// The macOS counterpart: exercise the real LaunchAgent provider against the
/// live `~/Library/LaunchAgents` + launchd domain and report PASS/FAIL,
/// restoring whatever state we found.
///
/// This is not redundant with `tests/macos_autostart.rs`. Everything there
/// that shells out to `launchctl` is `#[ignore]`d, because `HOME` cannot
/// redirect launchd's per-user domain (it is keyed by uid): `enable` would
/// bootstrap into the developer's live login session and `RunAtLoad` would
/// make launchd exec the test binary, while `disable` would boot out — and
/// kill — a Nudge they actually have running. So the launchctl path has no
/// tier that runs unattended. This command is how it gets exercised, by hand,
/// before shipping.
#[cfg(target_os = "macos")]
fn run_autostart_selftest() -> ! {
    use autostart::{AutostartProvider, LAUNCH_AGENT_LABEL, MacLaunchAgentProvider};

    macro_rules! fail {
        ($($a:tt)*) => {{ eprintln!("[selftest] FAIL: {}", format!($($a)*)); std::process::exit(1); }};
    }

    let provider = match MacLaunchAgentProvider::for_current_exe() {
        Ok(p) => p,
        Err(e) => fail!("{e}"),
    };
    println!("[selftest] label   {LAUNCH_AGENT_LABEL}");
    println!("[selftest] plist   {}", provider.plist_path().display());
    println!("[selftest] program {}", provider.program().display());

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

    // Idempotence is a real requirement here, not a nicety — the settings
    // checkbox can be re-affirmed at any time. It is also the step that would
    // catch a regression in `register`'s bootout-first ordering: without it,
    // launchd rejects the second bootstrap with an unclassifiable exit 5.
    if let Err(e) = provider.enable() {
        fail!("enable (second time, must be idempotent): {e}");
    }

    if let Err(e) = provider.disable() {
        fail!("disable: {e}");
    }
    let after_disable = provider.is_enabled().unwrap_or(true);
    println!("[selftest] after disable -> is_enabled = {after_disable} (want false)");

    // Leave the machine exactly as we found it.
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

    // Single-instance guard: the status-item menu spawns one process per
    // "Settings" click, and without this each click would stack another
    // window with its own unsynchronised copy of config.json.
    //
    // Both guards are held for the lifetime of the window — a named kernel
    // mutex on Windows, a `flock`ed file under Application Support on macOS —
    // and both mean the same thing when they refuse: someone else is already
    // showing this window, so exit silently. The linux dev host has neither
    // and needs neither; it has no menu to launch the settings UI from.
    #[cfg(target_os = "windows")]
    let _mutex_handle = match acquire_settings_singleton() {
        Some(h) => h,
        None => return Ok(()),
    };
    #[cfg(target_os = "macos")]
    let _settings_lock = match platform::acquire_settings_singleton() {
        Some(l) => l,
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

    // Provider: real registry on Windows, real LaunchAgent on macOS, an
    // in-memory fake elsewhere so the linux dev build compiles. Those users
    // don't actually reach the settings window (no tray to spawn it), so the
    // fake never runs in production.
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
    // Same refusal on macOS, for the same reason: the LaunchAgent provider is
    // the only thing that touches ~/Library/LaunchAgents, and substituting a
    // fake would flip the checkbox and persist `autostart: true` while launchd
    // never learns about the app.
    #[cfg(target_os = "macos")]
    let provider: Box<dyn autostart::AutostartProvider> = {
        match autostart::MacLaunchAgentProvider::for_current_exe() {
            Ok(p) => Box::new(p),
            Err(e) => {
                eprintln!("[nudge-settings] cannot construct autostart provider: {e}");
                return Err(eframe::Error::AppCreation(
                    format!("cannot construct autostart provider: {e}").into(),
                ));
            }
        }
    };
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let provider: Box<dyn autostart::AutostartProvider> =
        Box::new(autostart::FakeProvider::new(cfg.autostart));

    // Persistence closure: write atomically to the same config path the
    // main process reads from on launch. Move-captures the path; the
    // closure is held by SettingsApp for the lifetime of the window.
    let path_for_persist = config_path.clone();
    let persist: settings_app::PersistFn =
        Box::new(move |c: &config::Config| config::save(&path_for_persist, c));

    let win_size = [520.0_f32, 400.0];
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size(win_size)
        .with_min_inner_size([400.0, 320.0])
        .with_decorations(true)
        .with_resizable(true)
        .with_title("Nudge — Settings");
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    eframe::run_native(
        "Nudge — Settings",
        options,
        Box::new(move |cc| Ok(Box::new(SettingsApp::new(cc, cfg, provider, persist)))),
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> eframe::Result {
    use app::NudgeApp;
    use eframe::egui;

    // Manual QA hook — runs the autostart lifecycle against the real OS
    // (registry on Windows, LaunchAgent on macOS) and exits before any GUI /
    // config work. A no-op stub on platforms with no real provider.
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

    // Whatever OS integration this platform has, started before the event
    // loop takes the main thread.
    //
    // On Windows that is the tray thread: the tray icon, its menu and the
    // animation loop all live on one dedicated thread running its own
    // message pump, so animation keeps ticking even while eframe's popup
    // window is SW_HIDE'd. The same thread owns the global hotkey
    // registration (WM_HOTKEY is delivered to the registering thread's
    // message queue) and applies config-file live-reload.
    //
    // On macOS this only *records* what to build: the status item is an
    // `NSStatusItem` and the hotkey goes through Carbon, and both need the
    // main thread to already be an event loop. `platform::start_on_event_loop`
    // (below, in the creator closure) does the building.
    platform::start(Some(configured_hotkey), config_path.clone(), config.clone());

    // Live-reload: watch `config.json` for changes (settings UI saves,
    // or hand-edits) and ask the platform to re-read + diff — the tray
    // thread on Windows, `platform::tick` on macOS.
    // Failures are non-fatal (rare: directory missing, OS watch limit
    // hit) — the app still works, the user just doesn't get live
    // reload until the next restart.
    //
    // The handle MUST live for the rest of the process — dropping it
    // stops the watcher. eframe::run_native consumes the event loop
    // but local bindings here outlive the call, so a plain `let` is
    // enough; we name it `_watcher` (with the leading underscore) so
    // clippy doesn't flag the otherwise-unused binding.
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

    // Spotlight window: horizontally centered, vertical center at 40% of screen
    // per spec §1. Computed once at launch from primary monitor dimensions.
    let win_size = [
        nudge_state::POPUP_SIZE.0 as f32,
        nudge_state::POPUP_SIZE.1 as f32,
    ];
    #[cfg(target_os = "windows")]
    let viewport = {
        use windows::Win32::UI::WindowsAndMessaging::{GetSystemMetrics, SM_CXSCREEN, SM_CYSCREEN};
        let (screen_w, screen_h) = unsafe {
            (
                GetSystemMetrics(SM_CXSCREEN).max(1) as u32,
                GetSystemMetrics(SM_CYSCREEN).max(1) as u32,
            )
        };
        let (x, y) = nudge_state::window_position(
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
            .with_title("Nudge")
    };
    // macOS: same chrome as Windows minus the two Win32-shaped calls.
    // `with_taskbar(false)` has no macOS meaning — the Dock is an *app*
    // property, set through the activation policy below — and the position
    // cannot be set here at all, because the screen size is not knowable
    // before the event loop exists (`platform::macos::apply_window_position`
    // sends it on the first tick, in points).
    //
    // The rest is measured to work: transparency composites (the window's own
    // surface reads 0,0,0,0 at the corners while the card is opaque at the
    // centre), and `with_always_on_top` lands the window on level 3, floating
    // over an active Finder window.
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

    // No Dock icon and no Cmd-Tab entry: this is a menu-bar app whose window
    // is a transient overlay. `Accessory` is what `with_taskbar(false)` +
    // `WS_EX_TOOLWINDOW` do on Windows.
    //
    // It has to be asked for *before* the app finishes launching, which is why
    // it goes through the event-loop hook rather than a `setActivationPolicy`
    // call of our own — the latter would run after `applicationDidFinishLaunching`
    // and flash a Dock icon on the way past.
    //
    // An accessory app is not given the keyboard by default. What restores it
    // is the `activateIgnoringOtherApps` in `platform::macos::focus_popup`,
    // called on every reveal; without that call this line would produce a
    // popup nobody can type into.
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
            // The half of the OS integration that needs the main thread to
            // already be the event loop: the status item and the hotkey.
            platform::start_on_event_loop(&cc.egui_ctx);
            Ok(Box::new(NudgeApp::new(cc, default_minutes)))
        }),
    )
}

#[cfg(target_arch = "wasm32")]
fn main() {}
