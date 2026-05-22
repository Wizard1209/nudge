//! Idle CPU regression test for nudge.exe.
//!
//! Launches the real binary with a throwaway config (`default_interval_minutes`
//! = 1.0 — short enough that petal-animation drifts hit several times inside
//! our observation window), dismisses the initial popup via synthetic Esc,
//! then measures kernel+user CPU time over a 30-second window via
//! GetProcessTimes.
//!
//! Threshold ratchets DOWN as code improves: bump `THRESHOLD_MS_PER_SEC`
//! lower after each fix. Initial value is intentionally permissive — the
//! first run is a calibration. The follow-up tightens it to `actual + 20%`
//! and that becomes the regression guard.
//!
//! Why `#[ignore]`: ~35s wall time and only meaningful on Windows. Run with
//!
//! ```text
//! cargo test --target x86_64-pc-windows-gnu --test perf_idle -- --ignored --nocapture
//! ```

#![cfg(target_os = "windows")]

use std::path::PathBuf;
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
use windows::Win32::System::Threading::{
    GetProcessTimes, OpenProcess, TerminateProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_TERMINATE,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
    VK_ESCAPE,
};

/// Acceptable CPU-time per wall-second. 100 ms/s = 10% of one core — only
/// catches catastrophes. After the first calibration run, edit this down to
/// `(observed_baseline * 1.2)` and commit. Each subsequent optimization
/// lowers it further.
const THRESHOLD_MS_PER_SEC: f64 = 100.0;

/// Time after spawn before sending Esc. eframe init + tray-thread bootstrap
/// + window foregrounding — 3s is comfortably above worst-case cold-start.
const STARTUP_WAIT: Duration = Duration::from_secs(3);

/// Time after Esc before opening the measurement window. Lets hide_popup,
/// timer.reset, and the tray-thread's next park settle into steady state.
const SETTLE_AFTER_ESC: Duration = Duration::from_secs(2);

/// Observation window. At a 60s timer interval (12 petals → 5s/petal), this
/// catches six drift events — enough averaging to keep the measurement
/// repeatable. Also stays comfortably under the 60s interval, so the popup
/// never re-fires mid-measurement (which would be a wholly different code
/// path with much higher CPU).
const MEASURE_WINDOW: Duration = Duration::from_secs(30);

#[test]
#[ignore = "heavy: 35s wall time, Windows-only; run explicitly via --ignored"]
fn idle_cpu_is_under_threshold() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let config_path = tmp.path().join("config.json");
    std::fs::write(
        &config_path,
        r#"{"hotkey":"Ctrl+Shift+Space","default_interval_minutes":1.0}"#,
    )
    .expect("write throwaway config");

    let exe = PathBuf::from(env!("CARGO_BIN_EXE_nudge"));
    let child = Command::new(&exe)
        .arg("--config")
        .arg(&config_path)
        .spawn()
        .expect("spawn nudge.exe");

    // RAII: kill the child on any return path (panic, early return,
    // normal completion). Avoids a lingering nudge.exe if an assert fails.
    let mut guard = ChildGuard(child);
    let pid = guard.0.id();

    std::thread::sleep(STARTUP_WAIT);
    send_escape();
    std::thread::sleep(SETTLE_AFTER_ESC);

    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .expect("OpenProcess for measurement");

    let cpu_before = process_cpu_time(handle);
    let wall_start = Instant::now();
    std::thread::sleep(MEASURE_WINDOW);
    let cpu_after = process_cpu_time(handle);
    let wall_elapsed = wall_start.elapsed();

    unsafe {
        let _ = CloseHandle(handle);
    }

    // Kill before asserting so a failure doesn't leave nudge.exe behind
    // even with the RAII guard (defense in depth).
    let _ = guard.0.kill();
    let _ = guard.0.wait();

    let cpu_delta = cpu_after - cpu_before;
    let ms_per_sec = cpu_delta.as_secs_f64() * 1000.0 / wall_elapsed.as_secs_f64();

    eprintln!(
        "nudge.exe idle: {ms_per_sec:.3} ms/s CPU over {:.1}s (threshold {THRESHOLD_MS_PER_SEC:.3} ms/s)",
        wall_elapsed.as_secs_f64()
    );

    assert!(
        ms_per_sec < THRESHOLD_MS_PER_SEC,
        "idle CPU {ms_per_sec:.3} ms/s exceeds threshold {THRESHOLD_MS_PER_SEC:.3} ms/s — regression?"
    );
}

fn process_cpu_time(handle: HANDLE) -> Duration {
    let mut creation = FILETIME::default();
    let mut exit = FILETIME::default();
    let mut kernel = FILETIME::default();
    let mut user = FILETIME::default();
    unsafe {
        GetProcessTimes(handle, &mut creation, &mut exit, &mut kernel, &mut user)
            .expect("GetProcessTimes");
    }
    filetime_to_duration(kernel) + filetime_to_duration(user)
}

fn filetime_to_duration(ft: FILETIME) -> Duration {
    // FILETIME = number of 100-nanosecond intervals.
    let ticks = ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64);
    Duration::from_nanos(ticks.saturating_mul(100))
}

fn send_escape() {
    let make_event = |flags: KEYBD_EVENT_FLAGS| INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: VK_ESCAPE,
                wScan: 0,
                dwFlags: flags,
                time: 0,
                dwExtraInfo: 0,
            },
        },
    };
    let inputs = [make_event(KEYBD_EVENT_FLAGS(0)), make_event(KEYEVENTF_KEYUP)];
    let size = std::mem::size_of::<INPUT>() as i32;
    let sent = unsafe { SendInput(&inputs, size) };
    assert_eq!(sent, inputs.len() as u32, "SendInput failed to inject Esc");
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
        // Best-effort: if the test already killed cleanly, kill returns
        // an error we can ignore. We also brute-force terminate via the
        // PID in case the Child handle is in a weird state.
        let pid = self.0.id();
        let _ = self.0.kill();
        let _ = self.0.wait();
        unsafe {
            if let Ok(h) = OpenProcess(PROCESS_TERMINATE, false, pid) {
                let _ = TerminateProcess(h, 1);
                let _ = CloseHandle(h);
            }
        }
    }
}
