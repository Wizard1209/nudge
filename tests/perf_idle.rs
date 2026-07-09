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
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, TH32CS_SNAPTHREAD, THREADENTRY32, Thread32First, Thread32Next,
};
use windows::Win32::System::Threading::{
    GetProcessTimes, GetThreadDescription, GetThreadTimes, OpenProcess, OpenThread,
    PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_TERMINATE, THREAD_QUERY_LIMITED_INFORMATION,
    TerminateProcess,
};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP, SendInput,
    VK_ESCAPE,
};

/// Acceptable CPU-time per wall-second while idle (popup hidden).
///
/// The bug this guards against: a hidden eframe window busy-looped winit's
/// main thread at ~1000 ms/s (a full core) in release builds — eframe ≤0.33
/// would `request_redraw()` an invisible window, get no `RedrawRequested`
/// back, and leave `ControlFlow` stuck on `Poll`. eframe 0.34 fixed it by
/// painting invisible windows directly on a throttled interval; measured
/// idle is now effectively 0 ms/s here.
///
/// 25 ms/s = 2.5% of one core: ~40× headroom below the busy-loop, with slack
/// for the throttled invisible-window repaints and measurement noise. Do not
/// raise this back toward 100 without a reason — that was the calibration
/// value from before the fix landed.
const THRESHOLD_MS_PER_SEC: f64 = 25.0;

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
    let threads_before = enumerate_thread_cpu(pid);
    let wall_start = Instant::now();
    std::thread::sleep(MEASURE_WINDOW);
    let cpu_after = process_cpu_time(handle);
    let threads_after = enumerate_thread_cpu(pid);
    let wall_elapsed = wall_start.elapsed();

    // Per-thread CPU breakdown — points at the actual offending thread.
    let mut deltas: Vec<(u32, Duration)> = threads_after
        .iter()
        .map(|(tid, after)| {
            let before = threads_before.get(tid).copied().unwrap_or_default();
            (*tid, after.checked_sub(before).unwrap_or_default())
        })
        .collect();
    deltas.sort_by(|a, b| b.1.cmp(&a.1));
    eprintln!(
        "[perf-diag] per-thread CPU over {:.1}s window (top 10):",
        wall_elapsed.as_secs_f64()
    );
    for (tid, delta) in deltas.iter().take(10) {
        let ms = delta.as_secs_f64() * 1000.0;
        let ms_per_sec = ms / wall_elapsed.as_secs_f64();
        let name = thread_description(*tid).unwrap_or_else(|| "<unnamed>".to_string());
        eprintln!("[perf-diag]   tid={tid} \"{name}\": {ms:.1} ms total ({ms_per_sec:.1} ms/s)");
    }

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

/// Best-effort thread name via GetThreadDescription. Libraries that set
/// names via SetThreadDescription (Rust's std, tokio, winit, etc.) show
/// up here; OS threads typically don't.
fn thread_description(tid: u32) -> Option<String> {
    unsafe {
        let h = OpenThread(THREAD_QUERY_LIMITED_INFORMATION, false, tid).ok()?;
        let name_ptr = GetThreadDescription(h).ok();
        let _ = CloseHandle(h);
        let name_ptr = name_ptr?;
        if name_ptr.is_null() {
            return None;
        }
        let s = name_ptr.to_string().ok();
        // Intentionally not LocalFree'ing the buffer — diagnostic-only,
        // process exits shortly anyway.
        s.filter(|s| !s.is_empty())
    }
}

/// Walk every thread of the process and snapshot its accumulated
/// kernel+user CPU time. Returns a map of TID → total. Threads created
/// or destroyed mid-window are handled gracefully via the diff caller:
/// `unwrap_or_default()` on missing keys.
fn enumerate_thread_cpu(pid: u32) -> std::collections::HashMap<u32, Duration> {
    let mut times = std::collections::HashMap::new();
    let snapshot = match unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) } {
        Ok(h) => h,
        Err(_) => return times,
    };
    let mut entry = THREADENTRY32 {
        dwSize: std::mem::size_of::<THREADENTRY32>() as u32,
        ..Default::default()
    };
    unsafe {
        if Thread32First(snapshot, &mut entry).is_ok() {
            loop {
                if entry.th32OwnerProcessID == pid
                    && let Ok(h) =
                        OpenThread(THREAD_QUERY_LIMITED_INFORMATION, false, entry.th32ThreadID)
                {
                    let mut creation = FILETIME::default();
                    let mut exit = FILETIME::default();
                    let mut kernel = FILETIME::default();
                    let mut user = FILETIME::default();
                    if GetThreadTimes(h, &mut creation, &mut exit, &mut kernel, &mut user).is_ok() {
                        let total = filetime_to_duration(kernel) + filetime_to_duration(user);
                        times.insert(entry.th32ThreadID, total);
                    }
                    let _ = CloseHandle(h);
                }
                // Reset dwSize on each iteration — required by the API.
                entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
                if Thread32Next(snapshot, &mut entry).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snapshot);
    }
    times
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
    let inputs = [
        make_event(KEYBD_EVENT_FLAGS(0)),
        make_event(KEYEVENTF_KEYUP),
    ];
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
