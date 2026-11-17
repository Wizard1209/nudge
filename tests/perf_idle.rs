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

const THRESHOLD_MS_PER_SEC: f64 = 25.0;

const STARTUP_WAIT: Duration = Duration::from_secs(5);

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

    let mut guard = ChildGuard(child);
    let pid = guard.0.id();

    std::thread::sleep(STARTUP_WAIT);

    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }
        .expect("OpenProcess for measurement");

    let cpu_before = process_cpu_time(handle);
    let threads_before = enumerate_thread_cpu(pid);
    let wall_start = Instant::now();
    std::thread::sleep(MEASURE_WINDOW);
    let cpu_after = process_cpu_time(handle);
    let threads_after = enumerate_thread_cpu(pid);
    let wall_elapsed = wall_start.elapsed();

    let mut deltas: Vec<(u32, Duration)> = threads_after
        .iter()
        .map(|(tid, after)| {
            let before = threads_before.get(tid).copied().unwrap_or_default();
            (*tid, after.checked_sub(before).unwrap_or_default())
        })
        .collect();
    deltas.sort_by_key(|&(_, delta)| std::cmp::Reverse(delta));
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
        s.filter(|s| !s.is_empty())
    }
}

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
    let ticks = ((ft.dwHighDateTime as u64) << 32) | (ft.dwLowDateTime as u64);
    Duration::from_nanos(ticks.saturating_mul(100))
}

struct ChildGuard(Child);

impl Drop for ChildGuard {
    fn drop(&mut self) {
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
