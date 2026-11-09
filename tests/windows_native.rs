//! The spec §4 state machine, observed on Windows from outside the app.
//!
//! Each test launches the built `nudge.exe` with `--config` pointing at a
//! temporary file and then sees it only the way a user would: whether its
//! window is on screen and holds the foreground, what the journal on disk
//! says, and when the popup comes back. Input is synthetic (`SendInput`), the
//! hotkey included. Nothing from the crate is imported, so the file builds
//! unchanged against any tree that keeps the `--config` flag, the config keys
//! and the journal location.
//!
//! The journal cannot be redirected: the app writes to the Documents known
//! folder. [`Journal`] moves the user's file aside for the length of a test
//! and puts it back afterwards.
//!
//! Needs a logged-in desktop, so `#[ignore]`d. In the VM:
//! `SUITE=windows_native scripts/native-tests-windows.sh`.

#![cfg(target_os = "windows")]

use std::path::{Path, PathBuf};
use std::process::{Child, Command};
use std::time::{Duration, Instant};

use windows::Win32::Foundation::{HWND, LPARAM, RECT};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    INPUT, INPUT_0, INPUT_KEYBOARD, KEYBD_EVENT_FLAGS, KEYBDINPUT, KEYEVENTF_KEYUP,
    MAPVK_VK_TO_VSC, MapVirtualKeyW, SendInput, VIRTUAL_KEY, VK_CONTROL, VK_ESCAPE, VK_MENU,
    VK_OEM_PERIOD, VK_RETURN, VK_SHIFT, VK_SPACE, VK_TAB,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetForegroundWindow, GetWindowRect, GetWindowThreadProcessId, IsWindowVisible,
};
use windows::core::BOOL;

/// From spawn to a visible popup, cold start under x64 emulation included.
const LAUNCH_BUDGET: Duration = Duration::from_secs(15);
/// From a key press to the window it acts on being shown or hidden.
const REACTION: Duration = Duration::from_secs(3);
/// How long a popup that has just appeared gets before keys are sent to it:
/// the window is shown by the tray thread, and the app turns the show into
/// its own state on the next frame.
const SETTLE: Duration = Duration::from_millis(500);
/// Long enough for the app to have run a frame after a key. Keys that reach
/// it within one frame are handled in an order of its own: egui moves focus
/// on Tab at the start of the next frame, and the popup acts on Enter before
/// the field takes the text that came with it.
const FRAME: Duration = Duration::from_millis(250);
/// Allowed lateness of a timer-driven reappearance. The app fires 250 ms
/// after the deadline at most (the last petal's fall), then paints.
const LATE: Duration = Duration::from_secs(3);
/// Allowed earliness: none is legitimate, this is measurement slack.
const EARLY: Duration = Duration::from_millis(500);

const HOTKEY_DEFAULT: &[VIRTUAL_KEY] = &[VK_CONTROL, VK_SHIFT, VK_SPACE];
const HOTKEY_RELOADED: (&str, &[VIRTUAL_KEY]) = (
    "Ctrl+Alt+K",
    &[VK_CONTROL, VK_MENU, VIRTUAL_KEY(b'K' as u16)],
);

fn config_json(hotkey: &str, minutes: f64) -> String {
    format!(r#"{{"hotkey":"{hotkey}","default_interval_minutes":{minutes}}}"#)
}

fn secs(minutes: f64) -> Duration {
    Duration::from_secs_f64(minutes * 60.0)
}

// ---------- the app under test ------------------------------------------------

struct App {
    child: Child,
    config: PathBuf,
    _dir: tempfile::TempDir,
}

impl App {
    fn launch(config: &str) -> App {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.json");
        std::fs::write(&path, config).expect("write config");
        let child = Command::new(env!("CARGO_BIN_EXE_nudge"))
            .arg("--config")
            .arg(&path)
            .spawn()
            .expect("spawn nudge.exe");
        App {
            child,
            config: path,
            _dir: dir,
        }
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    /// The popup: a visible top-level window of this process larger than
    /// the tray's helper windows.
    fn popup(&self) -> Option<HWND> {
        visible_windows(self.pid())
            .into_iter()
            .find(|&(_, r)| r.right - r.left > 100 && r.bottom - r.top > 100)
            .map(|(h, _)| h)
    }

    fn is_shown(&self) -> bool {
        self.popup().is_some()
    }

    fn has_foreground(&self) -> bool {
        let fg = unsafe { GetForegroundWindow() };
        if fg.0.is_null() {
            return false;
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(fg, Some(&mut pid)) };
        pid == self.pid()
    }

    /// When the popup was first seen on screen, polling until `deadline`.
    fn shown_by(&self, deadline: Instant) -> Option<Instant> {
        poll_until(deadline, || self.is_shown())
    }

    fn hidden_by(&self, deadline: Instant) -> Option<Instant> {
        poll_until(deadline, || !self.is_shown())
    }

    /// Wait for the popup to be up and to hold the keyboard, then give the
    /// app its frame. Returns when it was first seen.
    fn wait_ready(&self, within: Duration, what: &str) -> Instant {
        let deadline = Instant::now() + within;
        let shown = self
            .shown_by(deadline)
            .unwrap_or_else(|| panic!("{what}: the popup did not appear within {within:?}"));
        poll_until(shown + REACTION, || self.has_foreground()).unwrap_or_else(|| {
            panic!("{what}: the popup is on screen but never took the foreground")
        });
        std::thread::sleep(SETTLE);
        shown
    }

    /// Esc, and the moment the popup was seen gone.
    fn leave(&self, what: &str) -> Instant {
        self.close_with(VK_ESCAPE, "Esc", what)
    }

    fn submit(&self, what: &str) -> Instant {
        self.close_with(VK_RETURN, "Enter", what)
    }

    fn close_with(&self, key: VIRTUAL_KEY, name: &str, what: &str) -> Instant {
        press(&[key]);
        if let Some(t) = self.hidden_by(Instant::now() + REACTION) {
            return t;
        }
        // Not a retry: the test fails either way. What a second press does
        // tells a popup that ignores keys from one that never got them.
        let foreground = self.has_foreground();
        press(&[key]);
        let second = self.hidden_by(Instant::now() + REACTION).is_some();
        panic!(
            "{what}: {name} did not hide the popup within {REACTION:?} \
             (popup held the foreground: {foreground}; a second {name} hid it: {second})"
        )
    }

    /// Press `hotkey` while the popup is hidden and wait for it to come up.
    fn summon(&self, hotkey: &[VIRTUAL_KEY], what: &str) -> Instant {
        assert!(!self.is_shown(), "{what}: the popup was already up");
        press(hotkey);
        self.wait_ready(REACTION, what)
    }

    /// Replace the config the way the settings window does: a whole new
    /// file renamed over the old one.
    fn rewrite_config(&self, config: &str) {
        let tmp = self.config.with_extension("json.tmp");
        std::fs::write(&tmp, config).expect("write config");
        std::fs::rename(&tmp, &self.config).expect("rename config");
    }
}

impl Drop for App {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn poll_until(deadline: Instant, mut cond: impl FnMut() -> bool) -> Option<Instant> {
    loop {
        if cond() {
            return Some(Instant::now());
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn visible_windows(pid: u32) -> Vec<(HWND, RECT)> {
    struct Data {
        pid: u32,
        found: Vec<(HWND, RECT)>,
    }
    extern "system" fn each(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let data = unsafe { &mut *(lparam.0 as *mut Data) };
        let mut owner = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, Some(&mut owner)) };
        if owner == data.pid && unsafe { IsWindowVisible(hwnd) }.as_bool() {
            let mut r = RECT::default();
            if unsafe { GetWindowRect(hwnd, &mut r) }.is_ok() {
                data.found.push((hwnd, r));
            }
        }
        BOOL(1)
    }
    let mut data = Data {
        pid,
        found: Vec::new(),
    };
    unsafe {
        let _ = EnumWindows(Some(each), LPARAM(&mut data as *mut Data as isize));
    }
    data.found
}

// ---------- keyboard ------------------------------------------------------------

fn key_input(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    let scan = unsafe { MapVirtualKeyW(vk.0 as u32, MAPVK_VK_TO_VSC) } as u16;
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 {
            ki: KEYBDINPUT {
                wVk: vk,
                wScan: scan,
                dwFlags: if up {
                    KEYEVENTF_KEYUP
                } else {
                    KEYBD_EVENT_FLAGS(0)
                },
                time: 0,
                dwExtraInfo: 0,
            },
        },
    }
}

/// Press a chord: every key down in order, then up in reverse.
fn press(chord: &[VIRTUAL_KEY]) {
    let mut inputs: Vec<INPUT> = chord.iter().map(|&vk| key_input(vk, false)).collect();
    inputs.extend(chord.iter().rev().map(|&vk| key_input(vk, true)));
    let sent = unsafe { SendInput(&inputs, std::mem::size_of::<INPUT>() as i32) };
    assert_eq!(sent as usize, inputs.len(), "SendInput refused the input");
}

/// Press a chord that changes where the next keys go (Tab, select-all), and
/// give the app a frame to apply it.
fn press_then_frame(chord: &[VIRTUAL_KEY]) {
    press(chord);
    std::thread::sleep(FRAME);
}

/// Type lowercase ASCII letters, digits, spaces and dots, by key code (a US
/// layout prints them unshifted), then give the app a frame to take them in.
fn type_text(text: &str) {
    for ch in text.chars() {
        let vk = match ch {
            'a'..='z' => VIRTUAL_KEY(ch.to_ascii_uppercase() as u16),
            '0'..='9' | ' ' => VIRTUAL_KEY(ch as u16),
            '.' => VK_OEM_PERIOD,
            _ => panic!("type_text cannot type {ch:?}"),
        };
        press(&[vk]);
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(FRAME);
}

// ---------- the journal ---------------------------------------------------------

/// The app's journal file, emptied for one test: an existing file is moved
/// aside and restored on drop. A backup left by a run that died before its
/// drop is the user's journal and is put back first.
struct Journal {
    path: PathBuf,
    backup: PathBuf,
}

impl Journal {
    fn isolate() -> Journal {
        let docs = dirs::document_dir().expect("Documents known folder");
        let path = docs.join("Nudge").join("journal-rust.ndjson");
        let backup = path.with_extension("ndjson.windows-native-backup");
        if backup.exists() {
            std::fs::rename(&backup, &path).expect("restore a stale journal backup");
        }
        if path.exists() {
            std::fs::rename(&path, &backup).expect("move the journal aside");
        }
        Journal { path, backup }
    }

    fn lines(&self) -> Vec<serde_json::Value> {
        read_lines(&self.path)
    }
}

impl Drop for Journal {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
        if self.backup.exists() {
            let _ = std::fs::rename(&self.backup, &self.path);
        }
    }
}

fn read_lines(path: &Path) -> Vec<serde_json::Value> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    text.lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| serde_json::from_str(l).unwrap_or_else(|e| panic!("journal line {l:?}: {e}")))
        .collect()
}

// ---------- assertions ------------------------------------------------------------

/// The popup came back `interval` after `from`, within the slack.
fn assert_back_after(from: Instant, at: Instant, interval: Duration, what: &str) {
    let took = at.duration_since(from);
    eprintln!(
        "[windows-native] {what}: back after {:.2}s (interval {:.2}s)",
        took.as_secs_f64(),
        interval.as_secs_f64()
    );
    assert!(
        took + EARLY >= interval && took <= interval + LATE,
        "{what}: the popup came back {:.2}s after, expected {:.2}s (+{:?} / -{:?})",
        took.as_secs_f64(),
        interval.as_secs_f64(),
        LATE,
        EARLY
    );
}

fn expect_back(app: &App, from: Instant, interval: Duration, what: &str) -> Instant {
    let at = app.shown_by(from + interval + LATE).unwrap_or_else(|| {
        panic!(
            "{what}: the popup did not come back within {:?}",
            interval + LATE
        )
    });
    assert_back_after(from, at, interval, what);
    at
}

// ---------- the tests -----------------------------------------------------------------

/// Asking·Timer is the initial state: launch shows the popup, holding the
/// keyboard.
#[test]
#[ignore = "drives a real desktop; run in the Windows VM via scripts/native-tests-windows.sh"]
fn launch_shows_the_popup_with_the_keyboard() {
    let app = App::launch(&config_json("Ctrl+Shift+Space", 10.0));
    app.wait_ready(LAUNCH_BUDGET, "launch");
    assert!(app.is_shown() && app.has_foreground());
}

/// Leave from Asking·Timer: timer := the minutes field — the field as the
/// user left it, not the configured default.
#[test]
#[ignore = "drives a real desktop; run in the Windows VM via scripts/native-tests-windows.sh"]
fn esc_from_a_timer_popup_rearms_it_from_the_minutes_field() {
    let app = App::launch(&config_json("Ctrl+Shift+Space", 0.05));
    app.wait_ready(LAUNCH_BUDGET, "launch");
    // Third row, whole text replaced: 0.15 min = 9 s against the config's 3 s.
    press_then_frame(&[VK_TAB]);
    press_then_frame(&[VK_TAB]);
    press_then_frame(&[VK_CONTROL, VIRTUAL_KEY(b'A' as u16)]);
    type_text("0.15");
    let hidden = app.leave("Esc from the launch popup");
    expect_back(&app, hidden, secs(0.15), "after Esc from a timer popup");
}

/// Submit with text from Asking·Timer: exactly one journal line, with the
/// typed answers, the trigger and the interval; and the timer re-armed from
/// the field.
#[test]
#[ignore = "drives a real desktop; run in the Windows VM via scripts/native-tests-windows.sh"]
fn submit_with_text_writes_one_journal_line_and_rearms() {
    let journal = Journal::isolate();
    let app = App::launch(&config_json("Ctrl+Shift+Space", 0.1));
    app.wait_ready(LAUNCH_BUDGET, "launch");
    type_text("writing tests");
    press_then_frame(&[VK_TAB]);
    type_text("no");
    let before = chrono::Utc::now();
    let hidden = app.submit("Enter with text");
    let after = chrono::Utc::now();

    let lines = journal.lines();
    assert_eq!(lines.len(), 1, "one Submit, one line: {lines:?}");
    let e = &lines[0];
    assert_eq!(e["schema_version"], 1, "{e}");
    assert_eq!(e["event_type"], "submitted", "{e}");
    assert_eq!(e["implementation"], "rust", "{e}");
    assert_eq!(
        e["trigger_source"], "timer",
        "the launch popup is a timer one: {e}"
    );
    assert_eq!(e["doing"], "writing tests", "{e}");
    assert_eq!(e["bullshit"], "no", "{e}");
    assert_eq!(e["next_interval_minutes"].as_f64(), Some(0.1), "{e}");
    assert!(
        e["entry_id"].as_str().is_some_and(|s| !s.is_empty()),
        "entry_id: {e}"
    );
    let captured = e["captured_at"].as_str().expect("captured_at is a string");
    let at = chrono::DateTime::parse_from_rfc3339(captured)
        .unwrap_or_else(|err| panic!("captured_at {captured:?} is not RFC 3339: {err}"));
    assert!(
        captured
            .split('.')
            .nth(1)
            .is_some_and(|frac| frac.len() >= 3 && frac[..3].bytes().all(|b| b.is_ascii_digit())),
        "captured_at {captured:?} carries no milliseconds"
    );
    let slack = chrono::Duration::seconds(2);
    assert!(
        at >= before - slack && at <= after + slack,
        "captured_at {captured} is not the moment of the Submit ({before} .. {after})"
    );

    expect_back(&app, hidden, secs(0.1), "after Submit");
    assert_eq!(journal.lines().len(), 1, "nothing else was written");
}

/// Submit with both answers empty writes nothing, and still re-arms.
#[test]
#[ignore = "drives a real desktop; run in the Windows VM via scripts/native-tests-windows.sh"]
fn submit_with_empty_answers_writes_nothing_and_rearms() {
    let journal = Journal::isolate();
    let app = App::launch(&config_json("Ctrl+Shift+Space", 0.1));
    app.wait_ready(LAUNCH_BUDGET, "launch");
    type_text("   ");
    let hidden = app.submit("Enter with blank answers");
    expect_back(&app, hidden, secs(0.1), "after an empty Submit");
    assert_eq!(
        journal.lines(),
        Vec::<serde_json::Value>::new(),
        "an empty Submit wrote a line"
    );
}

/// Summon by the hotkey while Waiting opens Asking·Manual; Leave from it does
/// not touch the timer, so the deadline set before the summon still fires on
/// time.
#[test]
#[ignore = "drives a real desktop; run in the Windows VM via scripts/native-tests-windows.sh"]
fn hotkey_summons_while_waiting_and_esc_keeps_the_deadline() {
    let interval = secs(0.2);
    let app = App::launch(&config_json("Ctrl+Shift+Space", 0.2));
    app.wait_ready(LAUNCH_BUDGET, "launch");
    let armed = app.leave("Esc from the launch popup");

    std::thread::sleep(Duration::from_secs(3));
    app.summon(HOTKEY_DEFAULT, "hotkey while waiting");
    std::thread::sleep((armed + Duration::from_secs(6)).saturating_duration_since(Instant::now()));
    let left = app.leave("Esc from the summoned popup");

    let back = app
        .shown_by(armed + interval + LATE)
        .expect("the deadline set before the summon never fired");
    assert_back_after(
        armed,
        back,
        interval,
        "deadline after a manual open and Esc",
    );
    assert!(
        back.duration_since(left) < interval - Duration::from_secs(2),
        "the popup waited a whole interval after the manual Esc: the timer was reset"
    );
}

/// A deadline that elapses behind Asking·Manual is still owed when the popup
/// is left, so it fires at once.
#[test]
#[ignore = "drives a real desktop; run in the Windows VM via scripts/native-tests-windows.sh"]
fn a_deadline_passed_behind_a_manual_popup_fires_when_it_is_left() {
    let interval = secs(0.1);
    let app = App::launch(&config_json("Ctrl+Shift+Space", 0.1));
    app.wait_ready(LAUNCH_BUDGET, "launch");
    let armed = app.leave("Esc from the launch popup");

    std::thread::sleep(Duration::from_millis(1500));
    app.summon(HOTKEY_DEFAULT, "hotkey while waiting");
    std::thread::sleep(
        (armed + interval + Duration::from_secs(3)).saturating_duration_since(Instant::now()),
    );
    assert!(
        app.is_shown(),
        "the manual popup closed by itself while the deadline passed"
    );
    let left = app.leave("Esc from the manual popup, deadline past");

    let back = app.shown_by(left + REACTION).unwrap_or_else(|| {
        panic!("the deadline that passed behind the manual popup did not fire within {REACTION:?} of leaving it")
    });
    eprintln!(
        "[windows-native] owed deadline fired {:.2}s after the Leave",
        back.duration_since(left).as_secs_f64()
    );
}

/// Live reload (§9): a new hotkey in config.json takes over at once — the new
/// combination summons, the old one no longer does — and what it opens is a
/// manual popup (§5: `trigger_source = manual`).
#[test]
#[ignore = "drives a real desktop; run in the Windows VM via scripts/native-tests-windows.sh"]
fn live_reload_moves_the_hotkey() {
    let journal = Journal::isolate();
    let app = App::launch(&config_json("Ctrl+Shift+Space", 10.0));
    app.wait_ready(LAUNCH_BUDGET, "launch");
    app.leave("Esc from the launch popup");

    app.summon(HOTKEY_DEFAULT, "the configured hotkey, before the reload");
    app.leave("Esc from the summoned popup");

    let (label, chord) = HOTKEY_RELOADED;
    app.rewrite_config(&config_json(label, 10.0));
    std::thread::sleep(Duration::from_secs(2));

    press(HOTKEY_DEFAULT);
    assert!(
        app.shown_by(Instant::now() + REACTION).is_none(),
        "the old hotkey still opens the popup after the config moved it to {label}"
    );
    app.summon(chord, &format!("the reloaded hotkey {label}"));
    type_text("reloaded");
    app.submit("Enter in the popup the new hotkey opened");

    let lines = journal.lines();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0]["trigger_source"], "manual", "{}", lines[0]);
    assert_eq!(lines[0]["doing"], "reloaded", "{}", lines[0]);
}

/// A manual open that lands within milliseconds of the deadline must not
/// lose the deadline: whichever of the two wins, nudges continue afterwards.
/// The hotkey is pressed at jittered offsets around the measured moment the
/// timer shows the popup, round after round; the app has to nudge again
/// after every round.
#[test]
#[ignore = "drives a real desktop; run in the Windows VM via scripts/native-tests-windows.sh"]
fn a_manual_open_at_the_deadline_does_not_stop_the_nudges() {
    const MINUTES: f64 = 0.04;
    const CALIBRATION: usize = 3;
    const ROUNDS: usize = 40;
    let interval = secs(MINUTES);
    let app = App::launch(&config_json("Ctrl+Shift+Space", MINUTES));
    app.wait_ready(LAUNCH_BUDGET, "launch");

    // When, after a Leave, the timer's popup appears.
    let mut fire = Vec::new();
    for i in 0..CALIBRATION {
        let left = app.leave(&format!("calibration {i}"));
        let back = expect_back(&app, left, interval, &format!("calibration {i}"));
        fire.push(back.duration_since(left));
        app.wait_ready(REACTION, &format!("calibration {i}"));
    }
    fire.sort();
    let fire_at = fire[CALIBRATION / 2];

    // Offsets from -50 ms to +10 ms around it, spread by a fixed LCG: the
    // hotkey lands in the last moments before the timer shows the popup, or
    // just after.
    let mut seed: u64 = 0x5eed_1e55;
    let mut offsets_ms = Vec::new();
    for round in 0..ROUNDS {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        let offset_ms = (seed >> 33) as i64 % 61 - 50;
        offsets_ms.push(offset_ms);

        let left = app.leave(&format!("round {round}"));
        let at = if offset_ms >= 0 {
            left + fire_at + Duration::from_millis(offset_ms as u64)
        } else {
            left + fire_at - Duration::from_millis((-offset_ms) as u64)
        };
        while Instant::now() < at {
            std::hint::spin_loop();
        }
        let was_up = app.is_shown();
        press(HOTKEY_DEFAULT);
        eprintln!(
            "[windows-native] round {round}: hotkey at {offset_ms:+} ms of the fire, popup {} at press",
            if was_up { "up" } else { "down" }
        );
        app.wait_ready(
            REACTION,
            &format!("round {round}: hotkey at {offset_ms:+} ms"),
        );
        let left = app.leave(&format!("round {round}: Esc after the hotkey"));

        // Up already when the hotkey went in: the summon was dropped and the
        // Leave re-armed the timer. Otherwise the deadline is either past
        // (fires at once) or still ahead of the manual popup.
        let owed = if was_up { interval } else { fire_at };
        let back = app.shown_by(left + owed + LATE);
        eprintln!(
            "[windows-native] round {round}: next nudge {}",
            back.map_or("NEVER".into(), |b| format!(
                "after {:.2}s",
                b.duration_since(left).as_secs_f64()
            ))
        );
        assert!(
            back.is_some(),
            "nudges stopped: after a manual open {offset_ms:+} ms from the deadline (round {round}) \
             the popup did not come back within {:?}; offsets so far {offsets_ms:?}",
            owed + LATE
        );
        app.wait_ready(REACTION, &format!("round {round}: the next nudge"));
    }
}
