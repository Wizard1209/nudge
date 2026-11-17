#![cfg(target_os = "macos")]

use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::CFString;
use core_graphics::display::{CGDisplay, kCGWindowListOptionAll};
use core_graphics::geometry::CGRect;
use core_graphics::window::{
    kCGWindowBounds, kCGWindowIsOnscreen, kCGWindowLayer, kCGWindowOwnerPID,
};

const BUDGET_HIDDEN_MB: f64 = 47.0;

const BUDGET_HIDDEN_AFTER_CYCLES_MB: f64 = 45.0;

const BUDGET_CYCLE_GROWTH_MB: f64 = 10.0;

const BUDGET_HIDDEN_CPU_MS_PER_S: f64 = 25.0;

const BUDGET_SHOW_LATENCY_MS: f64 = 1500.0;

const STARTUP_TIMEOUT: Duration = Duration::from_secs(10);

const KEY_TIMEOUT: Duration = Duration::from_secs(5);

const HIDE_TIMEOUT: Duration = Duration::from_secs(3);

const SETTLE: Duration = Duration::from_secs(2);

const HIDDEN_SAMPLES: usize = 30;

const STEADY_INTERVAL_MIN: f64 = 0.75;

const CYCLE_INTERVAL_MIN: f64 = 0.35;

/// From a hide to the first gap sample of the cycles launch. A hidden
/// window's memory goes back in steps, the last about 10 s after the hide;
/// a gap sampled before that reads the release, not what a re-show kept.
const CYCLE_SETTLE: Duration = Duration::from_secs(15);

const CYCLES: usize = 5;

const CYCLE_SAMPLES: usize = 3;

const HOTKEY: &str = "Ctrl+Alt+Shift+F19";

const MIB: f64 = 1024.0 * 1024.0;

#[test]
#[ignore = "macOS GUI session; ~4 min; takes the keyboard repeatedly — run by hand via --ignored"]
fn hidden_footprint_cpu_and_show_latency_stay_in_budget() {
    let steady = measure_steady();
    let cycles = measure_cycles();

    let after_first_reshow_mb = cycles.hidden_mb[1];
    let after_cycles_mb = *cycles.hidden_mb.last().expect("one gap per cycle");
    let growth = after_cycles_mb - after_first_reshow_mb;
    let worst = cycles
        .show_latency_ms
        .iter()
        .copied()
        .fold(f64::MIN, f64::max);

    let report = serde_json::json!({
        "binary": binary().display().to_string(),
        "profile": if cfg!(debug_assertions) { "debug" } else { "release" },
        "hide_method": "focus-loss via JXA self-activation",
        "startup_ms": steady.startup_ms,
        "hide_ms": steady.hide_ms,
        "visible_mb": steady.visible_mb,
        "hidden_mb": steady.hidden_mb,
        "hidden_cpu_ms_per_s": steady.cpu_ms_per_s,
        "hidden_wakeups_per_s": steady.wakeups_per_s,
        "hidden_interrupt_wakeups_per_s": steady.interrupt_wakeups_per_s,
        "lifetime_max_mb": steady.lifetime_max_mb,
        "iosurface_mb_hidden": steady.iosurface_mb,
        "unmapped_graphics_mb_hidden": steady.unmapped_graphics_mb,
        "cycles": CYCLES,
        "cycles_hidden_mb": cycles.hidden_mb,
        "hidden_after_cycles_mb": after_cycles_mb,
        "cycle_growth_mb": growth,
        "cycles_lifetime_max_mb": cycles.lifetime_max_mb,
        "show_latency_ms": cycles.show_latency_ms,
        "show_latency_ms_median": median(&cycles.show_latency_ms),
        "show_latency_ms_max": worst,
    });
    println!("PERF_MACOS_JSON {report}");

    assert!(
        steady.hidden_mb < BUDGET_HIDDEN_MB,
        "hidden footprint {:.1} MiB is over the {BUDGET_HIDDEN_MB} MiB budget",
        steady.hidden_mb
    );
    assert!(
        after_cycles_mb < BUDGET_HIDDEN_AFTER_CYCLES_MB,
        "hidden footprint after {CYCLES} re-shows {after_cycles_mb:.1} MiB is over the \
         {BUDGET_HIDDEN_AFTER_CYCLES_MB} MiB budget"
    );
    assert!(
        growth < BUDGET_CYCLE_GROWTH_MB,
        "re-shows 2..={CYCLES} grew the hidden footprint by {growth:.1} MiB \
         (budget {BUDGET_CYCLE_GROWTH_MB}); per gap: {:?}",
        cycles.hidden_mb
    );
    assert!(
        steady.cpu_ms_per_s < BUDGET_HIDDEN_CPU_MS_PER_S,
        "hidden CPU {:.2} ms/s is over the {BUDGET_HIDDEN_CPU_MS_PER_S} ms/s budget",
        steady.cpu_ms_per_s
    );
    assert!(
        worst < BUDGET_SHOW_LATENCY_MS,
        "a timer re-show took {worst:.0} ms from deadline to screen \
         (budget {BUDGET_SHOW_LATENCY_MS}); all: {:?}",
        cycles.show_latency_ms
    );
}

struct Steady {
    startup_ms: f64,
    hide_ms: f64,
    visible_mb: f64,
    hidden_mb: f64,
    cpu_ms_per_s: f64,
    wakeups_per_s: f64,
    interrupt_wakeups_per_s: f64,
    lifetime_max_mb: f64,
    iosurface_mb: Option<f64>,
    unmapped_graphics_mb: Option<f64>,
}

fn measure_steady() -> Steady {
    let launched = Instant::now();
    let app = SandboxedApp::launch(STEADY_INTERVAL_MIN);
    let pid = app.pid();
    let interval = Duration::from_secs_f64(STEADY_INTERVAL_MIN * 60.0);

    assert!(
        wait_until(interval + STARTUP_TIMEOUT, || popup_visible(pid)),
        "the first deadline's popup never came on screen within {STARTUP_TIMEOUT:?} \
         of the interval — is the session unlocked? app log: {}",
        app.log()
    );
    let startup_ms = (launched.elapsed().as_secs_f64() - interval.as_secs_f64()) * 1e3;
    wait_for_key(&app);
    std::thread::sleep(SETTLE);
    let visible_mb = sample(pid).footprint as f64 / MIB;

    let hide_ms = hide(&app).hide_ms();
    std::thread::sleep(SETTLE);

    let first = sample(pid);
    let started = Instant::now();
    let mut footprints = Vec::with_capacity(HIDDEN_SAMPLES);
    for i in 1..=HIDDEN_SAMPLES {
        sleep_until(started + Duration::from_secs(i as u64));
        assert!(
            !popup_visible(pid),
            "the popup came back during the hidden window — the interval is \
             longer, so something other than the timer showed it"
        );
        footprints.push(sample(pid).footprint as f64 / MIB);
    }
    let last = sample(pid);
    let wall = started.elapsed().as_secs_f64();

    let table = footprint_table(pid);
    eprintln!("---- footprint {pid} (hidden) ----\n{table}");
    let iosurface_mb = footprint_row_mb(&table, "IOSurface");
    let unmapped_graphics_mb =
        footprint_row_mb(&table, "Owned physical footprint (unmapped) (graphics)");

    let tb = timebase();
    let cpu_ns = tb.to_ns((last.user + last.system) - (first.user + first.system));
    Steady {
        startup_ms,
        hide_ms,
        visible_mb,
        hidden_mb: median(&footprints),
        cpu_ms_per_s: cpu_ns / 1e6 / wall,
        wakeups_per_s: ((last.idle_wakeups + last.interrupt_wakeups)
            - (first.idle_wakeups + first.interrupt_wakeups)) as f64
            / wall,
        interrupt_wakeups_per_s: (last.interrupt_wakeups - first.interrupt_wakeups) as f64 / wall,
        lifetime_max_mb: last.lifetime_max as f64 / MIB,
        iosurface_mb,
        unmapped_graphics_mb,
    }
}

struct Cycles {
    hidden_mb: Vec<f64>,
    lifetime_max_mb: f64,
    show_latency_ms: Vec<f64>,
}

fn measure_cycles() -> Cycles {
    let app = SandboxedApp::launch(CYCLE_INTERVAL_MIN);
    let pid = app.pid();
    let interval = Duration::from_secs_f64(CYCLE_INTERVAL_MIN * 60.0);

    assert!(
        wait_until(interval + STARTUP_TIMEOUT, || popup_visible(pid)),
        "the cycles launch never showed its first popup; app log: {}",
        app.log()
    );
    wait_for_key(&app);
    let mut hidden = hide(&app);
    let mut hidden_mb = vec![hidden_gap_footprint(pid)];

    let mut show_latency_ms = Vec::with_capacity(CYCLES);
    for n in 1..=CYCLES {
        assert!(
            wait_until(interval + STARTUP_TIMEOUT, || popup_visible(pid)),
            "cycle {n}: the timer never re-showed the popup; app log: {}",
            app.log()
        );
        let shown = Instant::now();
        let deadline = hidden.left + interval;
        show_latency_ms.push(if shown >= deadline {
            (shown - deadline).as_secs_f64() * 1e3
        } else {
            -((deadline - shown).as_secs_f64() * 1e3)
        });
        wait_for_key(&app);
        hidden = hide(&app);
        hidden_mb.push(hidden_gap_footprint(pid));
    }

    Cycles {
        hidden_mb,
        lifetime_max_mb: sample(pid).lifetime_max as f64 / MIB,
        show_latency_ms,
    }
}

fn hidden_gap_footprint(pid: u32) -> f64 {
    std::thread::sleep(CYCLE_SETTLE);
    let mut v = Vec::with_capacity(CYCLE_SAMPLES);
    for _ in 0..CYCLE_SAMPLES {
        assert!(
            !popup_visible(pid),
            "the popup re-showed inside a hidden gap — the gap no longer fits \
             in {CYCLE_INTERVAL_MIN} min"
        );
        v.push(sample(pid).footprint as f64 / MIB);
        std::thread::sleep(Duration::from_secs(1));
    }
    median(&v)
}

struct SandboxedApp {
    child: Child,
    home: tempfile::TempDir,
}

const CONFIG_SUBDIR: &str = "Library/Application Support/Nudge";
const LOG_FILE: &str = "nudge.log";

impl SandboxedApp {
    fn launch(interval_minutes: f64) -> Self {
        let home = tempfile::tempdir().expect("temp HOME");
        std::fs::create_dir_all(home.path().join("Documents")).expect("Documents");
        let config_dir = home.path().join(CONFIG_SUBDIR);
        std::fs::create_dir_all(&config_dir).expect("config dir");
        std::fs::write(
            config_dir.join("config.json"),
            format!(
                r#"{{"hotkey": "{HOTKEY}", "default_interval_minutes": {interval_minutes}, "autostart": false, "sprite": {{"enabled": false}}}}"#
            ),
        )
        .expect("seed config.json");
        let log = std::fs::File::create(home.path().join(LOG_FILE)).expect("log file");

        let child = Command::new(binary())
            .env("HOME", home.path())
            .stdout(Stdio::null())
            .stderr(log)
            .spawn()
            .expect("spawn nudge");
        Self { child, home }
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn log(&self) -> String {
        std::fs::read_to_string(self.home.path().join(LOG_FILE)).unwrap_or_default()
    }
}

impl Drop for SandboxedApp {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn binary() -> &'static Path {
    Path::new(env!("CARGO_BIN_EXE_nudge"))
}

fn wait_for_key(app: &SandboxedApp) {
    let pid = app.pid();
    assert!(
        wait_until(KEY_TIMEOUT, || frontmost_pid() == Some(pid)),
        "the popup never became the active application, so losing focus \
         cannot hide it; frontmost is pid {:?}",
        frontmost_pid()
    );
    std::thread::sleep(Duration::from_millis(300));
}

struct Hidden {
    left: Instant,
    off_screen: Instant,
}

impl Hidden {
    fn hide_ms(&self) -> f64 {
        (self.off_screen - self.left).as_secs_f64() * 1e3
    }
}

fn hide(app: &SandboxedApp) -> Hidden {
    let pid = app.pid();
    let mut activator = take_activation();
    let left = wait_until(HIDE_TIMEOUT, || frontmost_pid() != Some(pid));
    let left_at = Instant::now();
    let hidden = left && wait_until(HIDE_TIMEOUT, || !popup_visible(pid));
    let off_screen = Instant::now();
    let _ = activator.kill();
    let _ = activator.wait();
    assert!(
        left,
        "osascript never took activation from the popup within {HIDE_TIMEOUT:?}"
    );
    assert!(
        hidden,
        "the popup lost focus but was still on screen {HIDE_TIMEOUT:?} later; \
         app log: {}",
        app.log()
    );
    Hidden {
        left: left_at,
        off_screen,
    }
}

fn take_activation() -> Child {
    Command::new("osascript")
        .args([
            "-l",
            "JavaScript",
            "-e",
            r#"ObjC.import("AppKit"); const a = $.NSApplication.sharedApplication; a.setActivationPolicy($.NSApplicationActivationPolicyAccessory); a.activateIgnoringOtherApps(true); delay(2);"#,
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("osascript not runnable")
}

fn frontmost_pid() -> Option<u32> {
    let asn = Command::new("lsappinfo").arg("front").output().ok()?;
    let asn = String::from_utf8_lossy(&asn.stdout).trim().to_string();
    let info = Command::new("lsappinfo")
        .args(["info", "-only", "pid", &asn])
        .output()
        .ok()?;
    String::from_utf8_lossy(&info.stdout)
        .trim()
        .strip_prefix("\"pid\"=")?
        .parse()
        .ok()
}

fn popup_visible(pid: u32) -> bool {
    let Some(windows) = CGDisplay::window_list_info(kCGWindowListOptionAll, None) else {
        return false;
    };
    let key = |k| unsafe { CFString::wrap_under_get_rule(k) };
    let num = |d: &CFDictionary<CFString, CFType>, k| {
        d.find(key(k))
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|n| n.to_i64())
    };
    let mut best: Option<(f64, bool)> = None;
    for i in 0..windows.len() {
        let Some(entry) = windows.get(i) else {
            continue;
        };
        let d: CFDictionary<CFString, CFType> =
            unsafe { TCFType::wrap_under_get_rule(*entry as CFDictionaryRef) };
        if num(&d, unsafe { kCGWindowOwnerPID }) != Some(pid as i64)
            || num(&d, unsafe { kCGWindowLayer }).unwrap_or(0) == 0
        {
            continue;
        }
        let Some(b) = d
            .find(key(unsafe { kCGWindowBounds }))
            .and_then(|v| v.downcast::<CFDictionary>())
            .and_then(|b| CGRect::from_dict_representation(&b))
        else {
            continue;
        };
        let area = b.size.width * b.size.height;
        if area <= 0.0 {
            continue;
        }
        let on_screen = d
            .find(key(unsafe { kCGWindowIsOnscreen }))
            .and_then(|v| v.downcast::<CFBoolean>())
            .map(bool::from)
            .unwrap_or(false);
        if best.is_none_or(|(a, _)| area > a) {
            best = Some((area, on_screen));
        }
    }
    best.is_some_and(|(_, on_screen)| on_screen)
}

#[derive(Clone, Copy)]
struct Sample {
    footprint: u64,
    lifetime_max: u64,
    user: u64,
    system: u64,
    idle_wakeups: u64,
    interrupt_wakeups: u64,
}

fn sample(pid: u32) -> Sample {
    let mut ri: libc::rusage_info_v4 = unsafe { std::mem::zeroed() };
    let rc = unsafe {
        libc::proc_pid_rusage(
            pid as libc::c_int,
            libc::RUSAGE_INFO_V4,
            (&mut ri as *mut libc::rusage_info_v4).cast::<libc::rusage_info_t>(),
        )
    };
    assert_eq!(
        rc,
        0,
        "proc_pid_rusage({pid}) failed: {}",
        std::io::Error::last_os_error()
    );
    Sample {
        footprint: ri.ri_phys_footprint,
        lifetime_max: ri.ri_lifetime_max_phys_footprint,
        user: ri.ri_user_time,
        system: ri.ri_system_time,
        idle_wakeups: ri.ri_pkg_idle_wkups,
        interrupt_wakeups: ri.ri_interrupt_wkups,
    }
}

#[repr(C)]
struct Timebase {
    numer: u32,
    denom: u32,
}

impl Timebase {
    fn to_ns(&self, ticks: u64) -> f64 {
        ticks as f64 * self.numer as f64 / self.denom as f64
    }
}

unsafe extern "C" {
    fn mach_timebase_info(info: *mut Timebase) -> libc::c_int;
}

fn timebase() -> Timebase {
    let mut tb = Timebase { numer: 0, denom: 0 };
    let rc = unsafe { mach_timebase_info(&mut tb) };
    assert!(rc == 0 && tb.denom != 0, "mach_timebase_info failed");
    tb
}

fn footprint_table(pid: u32) -> String {
    Command::new("footprint")
        .arg(pid.to_string())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

fn footprint_row_mb(table: &str, category: &str) -> Option<f64> {
    table.lines().find_map(|line| {
        let mut words = line.split_whitespace();
        let value: f64 = words.next()?.parse().ok()?;
        let unit = words.next()?;
        let rest: Vec<&str> = words.skip(5).collect();
        if rest.join(" ") != category {
            return None;
        }
        let scale = match unit {
            "B" => 1.0 / MIB,
            "KB" => 1.0 / 1024.0,
            "MB" => 1.0,
            "GB" => 1024.0,
            _ => return None,
        };
        Some(value * scale)
    })
}

fn median(v: &[f64]) -> f64 {
    let mut s = v.to_vec();
    s.sort_by(f64::total_cmp);
    match s.len() {
        0 => f64::NAN,
        n if n % 2 == 1 => s[n / 2],
        n => (s[n / 2 - 1] + s[n / 2]) / 2.0,
    }
}

fn wait_until(timeout: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        if done() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn sleep_until(t: Instant) {
    if let Some(d) = t.checked_duration_since(Instant::now()) {
        std::thread::sleep(d);
    }
}
