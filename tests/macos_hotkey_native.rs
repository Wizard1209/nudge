//! The hotkey recorder against a real desktop, driven by real events.
//!
//! **Never run on your own machine.** Every test here registers a system-wide
//! combination and posts keystrokes into whatever has the focus. It belongs in
//! the Tart guest, which is what `scripts/native-tests-macos.sh` is for:
//!
//! ```sh
//! SUITE=macos_hotkey_native scripts/native-tests-macos.sh
//! SUITE=macos_hotkey_native scripts/native-tests-macos.sh -- records_the_spelled_combinations
//! ```
//!
//! # Why this file is also the fixture recorder
//!
//! The acceptance rule for this feature is that a test counts only if its
//! input is a recorded real event stream or a real keypress. That rule is
//! expensive to keep — a guest boot per assertion — so every test here runs
//! the settings binary with `NUDGE_RECORD_KEYS` set and copies the stream the
//! key monitor saw into `tests/fixtures/keystreams/`, with a provenance line
//! saying which machine, which layout, which keys and which test produced it.
//! `tests/hotkey_replay.rs` then replays those on every `cargo test`, in
//! milliseconds, on the host.
//!
//! One instrument, two tiers. No fixture exists that a real run did not
//! produce, and the fast tier cannot drift from the slow one, because the slow
//! one rewrites it.
//!
//! # How the assertions avoid passing for the wrong reason
//!
//! Three habits, because a recorder that never armed would otherwise satisfy
//! most of what is asserted here:
//!
//! - **The window is read, not guessed.** [`Ui::settings`] pulls the settings
//!   window's accessibility tree out of the running process, so the field, the
//!   hint under it, the Record/Cancel label and the save banner are all
//!   observed values rather than things inferred from `config.json`.
//! - **Every empty stream has a positive control.** A null assertion
//!   (`nothing was recorded`) is preceded, in the same process, by a chord
//!   that must land — so "the monitor saw nothing" is distinguishable from
//!   "the monitor is nothing".
//! - **The anchored key codes are literals.** Where a test's point is that the
//!   OS code and the §5 token agree, the code posted is a `kVK_*` constant
//!   spelled out here, never [`keycode::keycode_for`] — pressing through the
//!   table under test and reading back through the same table would hide a
//!   uniformly wrong row.
//!
//! # Its own test binary
//!
//! Separate from `tests/macos_native.rs` for the reason that file already
//! gives about platform globals, and because these tests hold machine-wide
//! registrations: `--test-threads=1`, one distinct combination per test, and
//! process death is what releases a binding.
//!
//! The window-geometry and synthetic-input helpers below overlap with that
//! file's. That is deliberate duplication rather than a shared harness: each
//! is a handful of lines of platform call, and the alternative — a common
//! module — would make a change for one suite able to break the other, which
//! is the property this port has been careful to avoid.

#![cfg(target_os = "macos")]

use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

use core_foundation::base::{CFType, TCFType};
use core_foundation::boolean::CFBoolean;
use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
use core_foundation::number::CFNumber;
use core_foundation::string::{CFString, CFStringRef};
use core_graphics::display::{CGDisplay, kCGWindowListOptionAll};
use core_graphics::event::{CGEvent, CGEventFlags, CGEventTapLocation, CGKeyCode};
use core_graphics::event_source::{CGEventSource, CGEventSourceStateID};
use core_graphics::geometry::CGRect as CGWindowRect;
use core_graphics::window::{
    kCGWindowBounds, kCGWindowIsOnscreen, kCGWindowLayer, kCGWindowOwnerPID,
};

use nudge_web::hotkey::{self, MOD_ALT, MOD_WIN};
use nudge_web::platform::{hotkey_carbon, keycode};
use nudge_web::recorder::PROMPT;

/// Long enough for eframe to open a window and run its first frames.
const STARTUP_GRACE: Duration = Duration::from_secs(4);
/// After a click or a chord, before reading what it did.
const SETTLE: Duration = Duration::from_millis(400);

const CONFIG_SUBDIR: &str = "Library/Application Support/Nudge";

// ---------------------------------------------------------------------------
// key codes, spelled out
// ---------------------------------------------------------------------------
//
// `kVK_*` from HIToolbox's `Events.h`, written here as literals on purpose:
// these are the anchors that tie the OS's own numbering to the §5 tokens
// `src/platform/macos/keycode.rs` claims for them. A test that pressed
// `keycode_for("J")` and then read the answer back through the same array
// would agree with itself no matter what the array said.

const VK_ANSI_C: CGKeyCode = 0x08;
const VK_ANSI_V: CGKeyCode = 0x09;
const VK_ANSI_J: CGKeyCode = 0x26;
const VK_ANSI_SLASH: CGKeyCode = 0x2C;
const VK_ANSI_KEYPAD5: CGKeyCode = 0x57;
const VK_ANSI_A: CGKeyCode = 0x00;
const VK_SPACE: CGKeyCode = 0x31;
const VK_TAB: CGKeyCode = 0x30;
const VK_ESCAPE: CGKeyCode = 0x35;
const VK_F13: CGKeyCode = 0x69;
const VK_LEFT_ARROW: CGKeyCode = 0x7B;
/// The modifier caps. A flags-changed event rides on one of these, so a chord
/// that is held has to name the cap it is held with.
const VK_COMMAND: CGKeyCode = 0x37;
const VK_SHIFT: CGKeyCode = 0x38;
const VK_OPTION: CGKeyCode = 0x3A;
const VK_CONTROL: CGKeyCode = 0x3B;

// ---------------------------------------------------------------------------
// the app under test
// ---------------------------------------------------------------------------

fn binary_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("target/debug/nudge")
}

fn fixture_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/keystreams")
}

/// A `nudge` process with its own HOME, its own `config.json`, and (for the
/// settings window) its own recording of every press edge the key monitor saw.
struct Sandboxed {
    child: Child,
    home: tempfile::TempDir,
    /// Where the key monitor is appending edges, when this is a settings
    /// window. The file may not exist if no key was ever pressed — an empty
    /// stream is a real result, and two acceptance bullets are exactly that.
    keystream: PathBuf,
}

impl Drop for Sandboxed {
    fn drop(&mut self) {
        // A leaked settings window swallows keys while armed, and a leaked
        // main instance holds a machine-wide binding. Either poisons the rest
        // of the run.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Sandboxed {
    fn seed(home: &Path, hotkey: &str) {
        let dir = home.join(CONFIG_SUBDIR);
        std::fs::create_dir_all(&dir).expect("config dir");
        std::fs::write(
            dir.join("config.json"),
            format!(
                r#"{{"hotkey": "{hotkey}", "default_interval_minutes": 60, "autostart": false}}"#
            ),
        )
        .expect("seed config.json");
    }

    /// The settings window, with a seeded hotkey and key recording on.
    fn settings(saved_hotkey: &str) -> Self {
        Self::settings_inner(saved_hotkey, false)
    }

    /// The same, with stderr captured so a test can assert the app logged no
    /// error. Read it with [`Sandboxed::stderr_after_exit`], which kills the
    /// child first — the pipe is never drained while the process runs.
    fn settings_logging(saved_hotkey: &str) -> Self {
        Self::settings_inner(saved_hotkey, true)
    }

    fn settings_inner(saved_hotkey: &str, capture_stderr: bool) -> Self {
        let home = tempfile::tempdir().expect("temp HOME");
        Self::seed(home.path(), saved_hotkey);
        let keystream = home.path().join("keystream.ndjson");
        let mut cmd = Command::new(binary_path());
        cmd.arg("--settings")
            .env("HOME", home.path())
            .env("NUDGE_RECORD_KEYS", &keystream);
        if capture_stderr {
            cmd.stderr(Stdio::piped());
        }
        let child = cmd
            .spawn()
            .expect("spawn nudge --settings — run `cargo build --bin nudge` first");
        std::thread::sleep(STARTUP_GRACE);
        Self {
            child,
            home,
            keystream,
        }
    }

    /// The main instance: the one that registers the hotkey. It reads no keys.
    fn main_instance(hotkey: &str) -> Self {
        let home = tempfile::tempdir().expect("temp HOME");
        Self::seed(home.path(), hotkey);
        std::fs::create_dir_all(home.path().join("Documents")).expect("Documents");
        let child = Command::new(binary_path())
            .env("HOME", home.path())
            .spawn()
            .expect("spawn nudge");
        std::thread::sleep(STARTUP_GRACE);
        Self {
            child,
            home,
            keystream: PathBuf::new(),
        }
    }

    /// Reuse this sandbox's HOME for a fresh process — the relaunch half of
    /// "the captured value survives Save and a relaunch".
    fn relaunch_settings(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.child = Command::new(binary_path())
            .arg("--settings")
            .env("HOME", self.home.path())
            .spawn()
            .expect("relaunch nudge --settings");
        std::thread::sleep(STARTUP_GRACE);
    }

    /// Start a *main* instance on this sandbox's HOME, so the value a
    /// recording just saved is the value that gets registered. Returned
    /// separately because it is a second live process, not a replacement.
    fn spawn_main_on_same_home(&self) -> Sandboxed {
        std::fs::create_dir_all(self.home.path().join("Documents")).expect("Documents");
        let child = Command::new(binary_path())
            .env("HOME", self.home.path())
            .spawn()
            .expect("spawn nudge");
        std::thread::sleep(STARTUP_GRACE);
        // A borrowed HOME: this handle must not own the TempDir, or dropping
        // it would delete the config out from under the other process.
        Sandboxed {
            child,
            home: tempfile::tempdir().expect("placeholder"),
            keystream: PathBuf::new(),
        }
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }

    fn config_path(&self) -> PathBuf {
        self.home.path().join(CONFIG_SUBDIR).join("config.json")
    }

    /// The `hotkey` field as it stands on disk.
    fn saved_hotkey(&self) -> String {
        self.config()["hotkey"]
            .as_str()
            .expect("hotkey field")
            .to_string()
    }

    fn config(&self) -> serde_json::Value {
        let text = std::fs::read_to_string(self.config_path()).expect("read config.json");
        serde_json::from_str(&text).expect("config.json is JSON")
    }

    /// The press edges the monitor recorded, verbatim.
    fn recorded_edges(&self) -> String {
        std::fs::read_to_string(&self.keystream).unwrap_or_default()
    }

    /// The edges as `(keycode, modifiers)`, which is what an assertion about
    /// *what the OS delivered* wants.
    fn edges(&self) -> Vec<(u64, u64)> {
        self.recorded_edges()
            .lines()
            .filter(|l| !l.trim().is_empty())
            .map(|l| {
                let v: serde_json::Value =
                    serde_json::from_str(l).unwrap_or_else(|e| panic!("edge {l:?}: {e}"));
                (
                    v["keycode"].as_u64().expect("keycode"),
                    v["modifiers"].as_u64().expect("modifiers"),
                )
            })
            .collect()
    }

    /// Start the stream over. Used between a positive control and the null
    /// case it protects, so the fixture landed for the null case holds only
    /// the null case.
    fn truncate_keystream(&self) {
        if self.keystream.exists() {
            std::fs::write(&self.keystream, "").expect("truncate keystream");
        }
    }

    /// Stop the process but keep the sandbox: the HOME lives in this struct,
    /// and dropping it would delete the config another process is reading.
    fn kill_child(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        std::thread::sleep(SETTLE);
    }

    /// Kill the child and return everything it wrote to stderr. Only valid on
    /// a sandbox made by [`Sandboxed::settings_logging`].
    fn stderr_after_exit(&mut self) -> String {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let mut text = String::new();
        if let Some(mut pipe) = self.child.stderr.take() {
            use std::io::Read;
            let _ = pipe.read_to_string(&mut text);
        }
        text
    }

    /// Land this run's stream in `tests/fixtures/keystreams/<bullet>.ndjson`,
    /// with the provenance the replay tier refuses to run without.
    ///
    /// `layout` is **read from the system**, not passed in: it is the one
    /// claim in the header a reader cannot check for themselves, so a literal
    /// here would be the harness asserting rather than observing.
    fn land_fixture(&self, bullet: &str, keys: &str, harness: &str) {
        let dir = fixture_dir();
        std::fs::create_dir_all(&dir).expect("fixture dir");
        let header = serde_json::json!({
            "bullet": bullet,
            "os": os_version(),
            "layout": current_input_source(),
            // Every stream here comes from `CGEventPost` into a real session:
            // the OS delivered it, which is what the rule asks for. A stream a
            // human typed would say `real-keyboard`.
            "input": "synthetic-cgevent",
            "keys": keys,
            "harness": harness,
        });
        let body = self.recorded_edges();
        std::fs::write(
            dir.join(format!("{bullet}.ndjson")),
            format!("{header}\n{body}"),
        )
        .expect("write fixture");
    }
}

fn os_version() -> String {
    let out = Command::new("sw_vers").output().expect("sw_vers");
    let text = String::from_utf8_lossy(&out.stdout);
    let field = |name: &str| {
        text.lines()
            .find_map(|l| l.strip_prefix(name))
            .map(|v| v.trim().to_string())
            .unwrap_or_default()
    };
    format!(
        "macOS {} ({})",
        field("ProductVersion:"),
        field("BuildVersion:")
    )
}

// ---------------------------------------------------------------------------
// the active keyboard layout, read and set
// ---------------------------------------------------------------------------
//
// Text Input Sources, called directly. The alternative — firing the
// "select previous input source" keystroke and trusting it — is an assertion
// that cannot fail informatively: `osascript` succeeds when the keystroke is
// *delivered*, whether or not a Russian source exists, is enabled, or became
// active. Measured in this guest: the keystroke leaves the layout on U.S.,
// while `TISSelectInputSource` returns 0 and the next read says Russian.

#[link(name = "Carbon", kind = "framework")]
unsafe extern "C" {
    fn TISCopyCurrentKeyboardInputSource() -> *const c_void;
    fn TISCreateInputSourceList(properties: *const c_void, include_all: bool) -> *const c_void;
    fn TISGetInputSourceProperty(source: *const c_void, key: CFStringRef) -> *const c_void;
    fn TISSelectInputSource(source: *const c_void) -> i32;
    static kTISPropertyInputSourceID: CFStringRef;
}

#[link(name = "CoreFoundation", kind = "framework")]
unsafe extern "C" {
    fn CFArrayGetCount(array: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
    fn CFRelease(value: *const c_void);
}

fn source_id(source: *const c_void) -> Option<String> {
    let raw = unsafe { TISGetInputSourceProperty(source, kTISPropertyInputSourceID) };
    if raw.is_null() {
        return None;
    }
    Some(unsafe { CFString::wrap_under_get_rule(raw as CFStringRef) }.to_string())
}

/// The input source that is active right now, e.g. `com.apple.keylayout.US`.
fn current_input_source() -> String {
    let source = unsafe { TISCopyCurrentKeyboardInputSource() };
    assert!(!source.is_null(), "no current keyboard input source");
    let id = source_id(source).expect("the current input source has an ID");
    unsafe { CFRelease(source) };
    id
}

/// Select an *enabled* input source by ID. Returns false if this guest does
/// not have it enabled, which is a precondition failure rather than a result.
fn select_input_source(id: &str) -> bool {
    let list = unsafe { TISCreateInputSourceList(std::ptr::null(), false) };
    assert!(!list.is_null(), "TISCreateInputSourceList returned nothing");
    let mut available = Vec::new();
    let mut selected = false;
    for i in 0..unsafe { CFArrayGetCount(list) } {
        let source = unsafe { CFArrayGetValueAtIndex(list, i) };
        let Some(sid) = source_id(source) else { continue };
        if sid == id {
            selected = unsafe { TISSelectInputSource(source) } == 0;
            break;
        }
        available.push(sid);
    }
    unsafe { CFRelease(list) };
    if !selected {
        eprintln!("[layout] {id} not selectable; enabled sources: {available:?}");
    }
    selected
}

/// Puts the layout back on the way out. The guest is a single shared VM, so a
/// test that switches the keyboard and leaves owes every later test a wrong
/// answer.
struct LayoutGuard(String);

impl LayoutGuard {
    /// Switch, then *read back*. The read is the assertion: it is what makes
    /// the provenance line's `layout` an observation.
    fn switch_to(id: &str) -> Self {
        let previous = current_input_source();
        assert!(
            select_input_source(id),
            "this guest has no enabled {id} — enable it in System Settings › Keyboard"
        );
        std::thread::sleep(SETTLE);
        assert_eq!(
            current_input_source(),
            id,
            "the input source did not actually change"
        );
        Self(previous)
    }
}

impl Drop for LayoutGuard {
    fn drop(&mut self) {
        select_input_source(&self.0);
    }
}

// ---------------------------------------------------------------------------
// reading the settings window
// ---------------------------------------------------------------------------

/// One accessibility element of the settings window: what it is, what it is
/// called, and what it holds.
#[derive(Debug, Clone)]
struct Element {
    role: String,
    name: String,
    value: String,
}

/// The AppleScript that walks to the settings window, wherever it is.
///
/// It **activates the window before reading it**, and that is load-bearing
/// rather than tidy-minded. Measured in this guest: a settings window that
/// opens while another application is frontmost never becomes key, eframe
/// therefore never runs a frame for it, and AccessKit — which builds its tree
/// on a frame — publishes nothing at all. The window is still listed by name,
/// with zero children, which reads exactly like "the recorder is broken" and
/// is not. The two bullets that hit this are the two that take the focus away
/// on purpose: focus-loss/`⌘Tab`, and the popup opening over the settings
/// window.
///
/// Which window to activate is decided by title, because in the two-process
/// bullet both windows are up at once: the popup's is exactly `Nudge`, the
/// settings window's is longer. Activating the popup instead — which is what
/// "the first process that has a window" picks once the popup is showing —
/// leaves the settings window unfocused, which is the very state this is here
/// to get out of.
///
/// **Not by process id.** Measured in this guest: with two `nudge` processes
/// alive, `first process whose unix id is <settings pid>` comes back holding
/// the *other* one — asked for 33604 it returned 33600, the windowless main
/// instance, and every `window 1 of p` after that failed with "Invalid index".
/// System Events' `whose` filter over processes is not trustworthy when two
/// share a name, and the two-process bullet is exactly the case that needs it.
///
/// So the window is found by what is *in* it: the only window in any `nudge`
/// process whose group contains a label reading `Global hotkey:`. The popup
/// has no such thing, and only one settings window is alive at a time — which
/// [`settings_group`]'s callers check rather than assume, by refusing a second
/// hit.
///
/// The tree also has to be asked for twice sometimes: AccessKit publishes it
/// on the first client request, so the first walk after a window opens finds
/// no group at all. Every caller retries.
fn settings_group(body: &str) -> String {
    format!(
        r#"tell application "System Events"
             repeat with attempt from 1 to 20
               set brought to false
               repeat with p in (every process whose name is "nudge")
                 repeat with w in (every window of p)
                   if name of w is not "Nudge" then
                     try
                       set frontmost of p to true
                     end try
                     set brought to true
                     exit repeat
                   end if
                 end repeat
                 if brought then exit repeat
               end repeat
               delay 0.3
               set hits to 0
               set answer to ""
               repeat with p in (every process whose name is "nudge")
                 repeat with w in (every window of p)
                   try
                     set g to group 1 of w
                     if (name of every UI element of g) contains "Global hotkey:" then
                       set hits to hits + 1
                       {body}
                     end if
                   end try
                 end repeat
               end repeat
               if hits > 1 then return "MANY"
               if hits is 1 then return answer
               delay 0.3
             end repeat
             return "NONE"
           end tell"#
    )
}

/// Run one of those scripts.
///
/// The retry lives inside the AppleScript — AccessKit publishes the tree on
/// the first client request, so the first walk after a window opens finds no
/// group at all, and re-asking in-process is cheaper than re-spawning
/// `osascript`. A second, outer retry covers `osascript` itself failing:
/// measured in this guest, it intermittently answers "not allowed assistive
/// access" (-25211) with nothing on stdout, and a bare stdout check would read
/// that as "no window" and say so in the panic.
///
/// `MANY` is never retried: two settings windows at once make every reading
/// ambiguous, and this suite's rule is one at a time.
fn ask_settings_window(body: &str) -> String {
    let script = settings_group(body);
    let mut last = String::new();
    for _ in 0..3 {
        let out = Command::new("osascript")
            .arg("-e")
            .arg(&script)
            .output()
            .expect("osascript not runnable");
        let answer = String::from_utf8_lossy(&out.stdout).to_string();
        let trimmed = answer.trim().to_string();
        assert_ne!(
            trimmed, "MANY",
            "two settings windows are open at once, so no reading of \"the\" \
             window means anything — drop one before opening the next"
        );
        if !trimmed.is_empty() && trimmed != "NONE" {
            return answer;
        }
        last = format!(
            "stdout {:?}, stderr {:?}",
            trimmed,
            String::from_utf8_lossy(&out.stderr).trim()
        );
        eprintln!("[ax] {last}; windows now:\n{}", nudge_windows());
        std::thread::sleep(Duration::from_millis(500));
    }
    panic!(
        "no settings window exposed accessibility children ({last}).\n\
         Live nudge processes and their windows:\n{}",
        nudge_windows()
    );
}

/// What the accessibility API can see of every `nudge` right now. Only ever
/// called to explain a failure — a panic that says "no window" is worth little
/// without saying what there was instead.
fn nudge_windows() -> String {
    let out = Command::new("osascript")
        .arg("-e")
        .arg(
            r#"tell application "System Events"
                 set out to ""
                 repeat with p in (every process whose name is "nudge")
                   set out to out & (unix id of p) & ": " & (name of every window of p) & linefeed
                 end repeat
                 if out is "" then return "(no process named nudge)"
                 return out
               end tell"#,
        )
        .output();
    match out {
        Ok(out) => format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
        Err(e) => format!("(could not ask: {e})"),
    }
}

/// The settings window's contents, as the accessibility API reports them.
///
/// eframe links AccessKit, and everything the recorder puts on screen — the
/// field, the Record/Cancel label, the hint under the row, the save banner —
/// comes back through it. Measured in this guest: the widgets are children of
/// an `AXGroup` inside the window rather than of the window itself, and
/// `entire contents` does not descend into that group, so the group is walked
/// explicitly.
struct Ui(Vec<Element>);

impl Ui {
    fn settings() -> Self {
        let text = ask_settings_window(
            r#"set answer to ""
                     repeat with e in (every UI element of g)
                       set r to "?"
                       try
                         set r to role of e as text
                       end try
                       set n to ""
                       try
                         set n to name of e as text
                       end try
                       set v to ""
                       try
                         set v to value of e as text
                       end try
                       set answer to answer & r & tab & n & tab & v & linefeed
                     end repeat"#,
        );
        Self(
            text.lines()
                .filter(|l| !l.trim().is_empty())
                .map(|line| {
                    let mut parts = line.splitn(3, '\t');
                    Element {
                        role: parts.next().unwrap_or_default().to_string(),
                        name: parts.next().unwrap_or_default().to_string(),
                        value: parts.next().unwrap_or_default().to_string(),
                    }
                })
                .collect(),
        )
    }

    fn index_of_role(&self, role: &str) -> Option<usize> {
        self.0.iter().position(|e| e.role == role)
    }

    /// The Record/Cancel button of the hotkey row: the first button in the
    /// window, ahead of Save and the window-level Cancel.
    fn row_button(&self) -> &Element {
        let i = self
            .index_of_role("AXButton")
            .unwrap_or_else(|| panic!("no button in the settings window: {:#?}", self.0));
        &self.0[i]
    }

    /// Whether the recorder is listening, read off the button's own label.
    fn armed(&self) -> bool {
        self.row_button().name == "Cancel"
    }

    /// The line under the hotkey row: the prompt while armed, a refusal naming
    /// the key after an unbindable press, the standing vocabulary when idle.
    fn hint(&self) -> String {
        let i = self.index_of_role("AXButton").expect("row button");
        self.0
            .get(i + 1)
            .filter(|e| e.role == "AXStaticText")
            .unwrap_or_else(|| panic!("no hint under the hotkey row: {:#?}", self.0))
            .name
            .clone()
    }

    /// The hotkey text field's contents. `None` while armed — the field is
    /// replaced by the prompt then, because a text box under "press a
    /// combination" reads as somewhere to type.
    fn hotkey_field(&self) -> Option<String> {
        let i = self.index_of_role("AXButton").expect("row button");
        let field = self.0.get(i.checked_sub(1)?)?;
        (field.role == "AXTextField").then(|| field.value.clone())
    }

    fn interval_field(&self) -> String {
        let i = self
            .0
            .iter()
            .position(|e| e.name.starts_with("Default interval"))
            .unwrap_or_else(|| panic!("no interval row: {:#?}", self.0));
        let field = &self.0[i + 1];
        assert_eq!(field.role, "AXTextField", "{:#?}", self.0);
        field.value.clone()
    }

    /// The status line above the Save button, when there is one.
    fn banner(&self) -> Option<String> {
        let i = self.index_of_role("AXCheckBox")?;
        self.0
            .get(i + 1)
            .filter(|e| e.role == "AXStaticText")
            .map(|e| e.name.clone())
    }
}

/// Click a named button in the settings window.
///
/// Addresses the button rather than a point, which is what makes this survive
/// a layout change. Measured in this guest: the widgets sit in an `AXGroup`
/// under the window, so `button of window 1` — which is what an earlier
/// version of this helper asked for — finds only the close/minimise/zoom caps
/// and fails with "Invalid index".
///
/// While armed there are two buttons called `Cancel`: the row's and the
/// window's. `first ... whose name is` takes the row's, which is the one that
/// means [`Input::Cancel`](nudge_web::recorder::Input::Cancel).
fn click_button(name: &str) {
    let answer = ask_settings_window(&format!(
        r#"set frontmost of p to true
                     click (first button of g whose name is "{name}")
                     set answer to "ok""#
    ));
    assert_eq!(
        answer.trim(),
        "ok",
        "could not click the {name:?} button"
    );
    std::thread::sleep(SETTLE);
}

/// Put the keyboard focus in the interval field.
///
/// By setting `AXFocused`, not by clicking: AccessKit publishes no `AXPress`
/// action on a text field, so `click` on one raises an error rather than
/// focusing it. The flag is read back, so a helper that quietly did nothing
/// would fail here rather than in the assertion it was setting up.
fn focus_interval_field() {
    let answer = ask_settings_window(
        r#"set f to last text field of g
                     set frontmost of p to true
                     set focused of f to true
                     delay 0.3
                     if focused of f then
                       set answer to "ok"
                     else
                       set answer to "not focused"
                     end if"#,
    );
    assert_eq!(
        answer.trim(),
        "ok",
        "the interval field never took the focus, so a paste aimed at it proves nothing"
    );
    std::thread::sleep(SETTLE);
}

/// Click Record and prove it armed before anything is pressed at it.
///
/// Every bullet below runs through here, so the one thing all of them depend
/// on is checked in all of them: the button existed, the click landed, the
/// state changed, and the row is asking for a combination.
fn arm() {
    click_button("Record");
    let ui = Ui::settings();
    assert!(
        ui.armed(),
        "the Record click did not arm the recorder: {:#?}",
        ui.0
    );
    assert_eq!(ui.hint(), PROMPT, "armed, but not asking for a combination");
}

// ---------------------------------------------------------------------------
// finding and driving the window
// ---------------------------------------------------------------------------

/// Whether this process is **showing the popup** right now.
///
/// Three filters, and every one of them is load-bearing. Measured in this
/// guest: a running `nudge` owns an unnamed 500×500 layer-0 surface that is
/// on-screen from launch and stays on-screen while the popup is hidden, so a
/// predicate that only asked "does this pid have a visible window" answers yes
/// before anything was pressed — which is how this helper was wrong the first
/// time it ran. The popup is built `.with_always_on_top()`, which macOS
/// reports as a non-zero `kCGWindowLayer`; that is the signal that tells the
/// two apart. `tests/macos_native.rs` reaches the same conclusion by the same
/// measurement, independently.
///
/// `kCGWindowListOptionAll` also keeps listing a window the app has ordered
/// out, so finding an entry is not visibility — `kCGWindowIsOnscreen` is. And
/// zero-size surfaces are skipped: the accessibility bridge owns some, and
/// they are never anything a user sees.
fn popup_visible(pid: u32) -> bool {
    window_present(pid, true)
}

/// Whether this process has any ordinary window presented — how the relaunch
/// test sees that the settings window came back up.
fn window_visible(pid: u32) -> bool {
    window_present(pid, false)
}

fn window_present(pid: u32, require_elevated: bool) -> bool {
    let Some(windows) = CGDisplay::window_list_info(kCGWindowListOptionAll, None) else {
        return false;
    };
    let owner_pid_key = unsafe { CFString::wrap_under_get_rule(kCGWindowOwnerPID) };
    let bounds_key = unsafe { CFString::wrap_under_get_rule(kCGWindowBounds) };
    let on_screen_key = unsafe { CFString::wrap_under_get_rule(kCGWindowIsOnscreen) };

    for i in 0..windows.len() {
        let Some(entry) = windows.get(i) else { continue };
        let dict: CFDictionary<CFString, CFType> =
            unsafe { TCFType::wrap_under_get_rule(*entry as CFDictionaryRef) };

        let owner = dict
            .find(&owner_pid_key)
            .and_then(|v| v.downcast::<CFNumber>())
            .and_then(|n| n.to_i64());
        if owner != Some(pid as i64) {
            continue;
        }
        let Some(bounds) = dict
            .find(&bounds_key)
            .and_then(|v| v.downcast::<CFDictionary>())
            .and_then(|d| CGWindowRect::from_dict_representation(&d))
        else {
            continue;
        };
        if bounds.size.width <= 0.0 || bounds.size.height <= 0.0 {
            continue;
        }
        let on_screen = dict
            .find(&on_screen_key)
            .and_then(|v| v.downcast::<CFBoolean>())
            .map(bool::from)
            .unwrap_or(false);
        if !on_screen {
            continue;
        }
        if require_elevated {
            let layer_key = unsafe { CFString::wrap_under_get_rule(kCGWindowLayer) };
            let elevated = dict
                .find(&layer_key)
                .and_then(|v| v.downcast::<CFNumber>())
                .and_then(|n| n.to_i64())
                .is_some_and(|layer| layer != 0);
            if !elevated {
                continue;
            }
        }
        return true;
    }
    false
}

/// Dismiss the popup Nudge shows the moment it launches, and wait until it is
/// really gone.
///
/// Measured, and pinned independently by `tests/macos_native.rs`
/// (`popup_reappears_after_the_interval` asserts it): a fresh main instance
/// puts its popup up straight away. So "the popup is visible" only means "the
/// hotkey fired" *after* the startup one has been cleared — without this, the
/// two-process bullets would be asserting something that was already true.
///
/// Call it while this is the only live `nudge`: the Esc is posted to whatever
/// has the focus, and a settings window would take it and close.
fn quiesce_popup(pid: u32) {
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    while popup_visible(pid) && std::time::Instant::now() < deadline {
        press_chord(VK_ESCAPE, CGEventFlags::CGEventFlagNull);
    }
    assert!(
        !popup_visible(pid),
        "the startup popup would not go away, so nothing below could tell it \
         apart from a popup the hotkey opened"
    );
}

fn event_source() -> CGEventSource {
    CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .expect("CGEventSourceCreate failed — is there a login session?")
}

/// Press and release one physical key with `modifiers` held — a press edge as
/// the OS delivers it, which is the only keyboard input an armed recorder
/// accepts.
///
/// The flags go on both edges: AppKit reads `modifierFlags` off the key-down
/// event itself, not off a separate flags-changed event, and a key-down with
/// no flags is a different combination.
///
/// The modifier caps are pressed and **released** around them anyway, each as
/// its own flags-changed edge, because the window server keeps its own idea of
/// what is held and the flags on a key event never tell it that a modifier
/// went up. Measured: a `⌘Tab` posted as one flagged key edge leaves the
/// application switcher on screen and Command stuck down — and this guest is a
/// single shared VM, so every test after it inherits a chord nobody pressed.
/// One red turns into six that way.
fn press_chord(code: CGKeyCode, modifiers: CGEventFlags) {
    let caps: Vec<(CGEventFlags, CGKeyCode)> = [
        (CGEventFlags::CGEventFlagCommand, VK_COMMAND),
        (CGEventFlags::CGEventFlagShift, VK_SHIFT),
        (CGEventFlags::CGEventFlagAlternate, VK_OPTION),
        (CGEventFlags::CGEventFlagControl, VK_CONTROL),
    ]
    .into_iter()
    .filter(|(flag, _)| modifiers.contains(*flag))
    .collect();

    // Down, one cap at a time, each event carrying everything held so far.
    let mut held = CGEventFlags::CGEventFlagNull;
    for (flag, cap) in &caps {
        held |= *flag;
        post_flags_changed(*cap, held);
    }
    for down in [true, false] {
        let event =
            CGEvent::new_keyboard_event(event_source(), code, down).expect("create key event");
        event.set_flags(modifiers);
        event.post(CGEventTapLocation::HID);
        std::thread::sleep(Duration::from_millis(60));
    }
    // Up, in reverse, ending on nothing held.
    for (flag, cap) in caps.iter().rev() {
        held.remove(*flag);
        post_flags_changed(*cap, held);
    }
    std::thread::sleep(SETTLE);
}

/// One modifier cap changing state: a key event on the cap's own key code,
/// retyped as `flagsChanged` and carrying the flags that are held afterwards.
fn post_flags_changed(cap: CGKeyCode, held: CGEventFlags) {
    let event = CGEvent::new_keyboard_event(event_source(), cap, true).expect("create event");
    event.set_flags(held);
    event.set_type(core_graphics::event::CGEventType::FlagsChanged);
    event.post(CGEventTapLocation::HID);
    std::thread::sleep(Duration::from_millis(60));
}

/// The same, addressed the way a person would: `⌥⌘J`, through the §5 parser
/// and the one key-code table the app registers with.
///
/// Used where the *combination* is the subject and the key code is incidental.
/// Where the key code is the subject, [`press_chord`] with a `VK_*` literal is
/// used instead, so the press cannot inherit the same mistake as the read.
fn press(label: &str) {
    let hk = hotkey::parse(label).expect("the label must parse");
    let code = keycode::keycode_for(hk.key.as_str()).expect("macOS must have a code for it");
    press_chord(code, carbon_to_cg_flags(hk.modifiers));
}

fn carbon_to_cg_flags(modifiers: u8) -> CGEventFlags {
    let mut flags = CGEventFlags::CGEventFlagNull;
    if modifiers & hotkey::MOD_CTRL != 0 {
        flags |= CGEventFlags::CGEventFlagControl;
    }
    if modifiers & MOD_ALT != 0 {
        flags |= CGEventFlags::CGEventFlagAlternate;
    }
    if modifiers & hotkey::MOD_SHIFT != 0 {
        flags |= CGEventFlags::CGEventFlagShift;
    }
    if modifiers & MOD_WIN != 0 {
        flags |= CGEventFlags::CGEventFlagCommand;
    }
    flags
}

/// `⌥⌘`, the workhorse chord of this suite.
fn alt_cmd() -> CGEventFlags {
    CGEventFlags::CGEventFlagAlternate | CGEventFlags::CGEventFlagCommand
}

/// Hold `modifiers` down and release them, pressing no key. Must produce no
/// event the recorder can see: modifier chords arrive as `flagsChanged`, which
/// the monitor's mask excludes.
fn hold_modifiers_only(modifiers: CGEventFlags) {
    for flags in [modifiers, CGEventFlags::CGEventFlagNull] {
        post_flags_changed(VK_COMMAND, flags);
    }
    std::thread::sleep(SETTLE);
}

/// Take the focus away from the settings window by activating another app.
fn steal_focus() {
    let _ = Command::new("osascript")
        .arg("-e")
        .arg(r#"tell application "Finder" to activate"#)
        .status();
    std::thread::sleep(SETTLE);
}

fn set_pasteboard(text: &str) {
    use std::io::Write;
    let mut child = Command::new("pbcopy")
        .stdin(Stdio::piped())
        .spawn()
        .expect("pbcopy");
    child
        .stdin
        .as_mut()
        .expect("pbcopy stdin")
        .write_all(text.as_bytes())
        .expect("write to pbcopy");
    child.wait().expect("pbcopy");
}

fn pasteboard() -> String {
    let out = Command::new("pbpaste").output().expect("pbpaste");
    String::from_utf8_lossy(&out.stdout).to_string()
}

// ---------------------------------------------------------------------------
// the instrument, before the bullets
// ---------------------------------------------------------------------------

/// Everything below depends on being able to see the settings window and press
/// its buttons. If that is broken, the bullets fail for a reason that has
/// nothing to do with the recorder, so it is checked once, on its own, first.
#[test]
#[ignore = "needs a real desktop; run via scripts/native-tests-macos.sh"]
fn the_settings_window_can_be_read_and_clicked() {
    // Bound, not dropped: this handle is what keeps the window open and kills
    // it afterwards. Nothing reads it, because the window is addressed by what
    // is in it rather than by which process owns it.
    let _app = Sandboxed::settings("Ctrl+Shift+Space");

    let idle = Ui::settings();
    assert!(!idle.armed(), "a fresh window must not be armed: {:#?}", idle.0);
    assert_eq!(
        idle.hotkey_field().as_deref(),
        Some("⌃⇧Space"),
        "the field must show the saved hotkey: {:#?}",
        idle.0
    );
    assert_eq!(idle.interval_field(), "60");
    assert_eq!(idle.banner(), None, "a fresh window has nothing to report");

    click_button("Record");
    let armed = Ui::settings();
    assert!(armed.armed(), "Record did not arm: {:#?}", armed.0);
    assert_eq!(armed.hint(), PROMPT);
    assert_eq!(
        armed.hotkey_field(),
        None,
        "the field is hidden while armed: {:#?}",
        armed.0
    );

    click_button("Cancel");
    let back = Ui::settings();
    assert!(!back.armed(), "Cancel did not disarm: {:#?}", back.0);
    assert_eq!(back.hotkey_field().as_deref(), Some("⌃⇧Space"));
}

// ---------------------------------------------------------------------------
// the acceptance bullets
// ---------------------------------------------------------------------------

/// `⌥⌘J`, `⌥⌘←` and `⇧/` land in the field spelled the way §5 writes them.
///
/// The key codes are `kVK_*` literals rather than table lookups, so the press
/// and the read cannot share a mistake: if `keycode.rs` gave `←` the wrong
/// code, the arrow would still be pressed here and the field would come back
/// wrong.
#[test]
#[ignore = "needs a real desktop; run via scripts/native-tests-macos.sh"]
fn records_the_spelled_combinations() {
    for (bullet, code, flags, expected) in [
        ("alt-cmd-j", VK_ANSI_J, alt_cmd(), "⌥⌘J"),
        ("alt-cmd-left", VK_LEFT_ARROW, alt_cmd(), "⌥⌘←"),
        (
            "shift-slash",
            VK_ANSI_SLASH,
            CGEventFlags::CGEventFlagShift,
            "⇧/",
        ),
    ] {
        let app = Sandboxed::settings("Ctrl+Shift+Space");
        arm();
        press_chord(code, flags);

        let ui = Ui::settings();
        assert!(!ui.armed(), "{expected} did not end the recording: {:#?}", ui.0);
        assert_eq!(
            ui.hotkey_field().as_deref(),
            Some(expected),
            "{expected}: the field must read what §5 spells"
        );
        assert_eq!(
            app.edges(),
            vec![(code as u64, ui_modifiers(flags))],
            "{expected}: the monitor saw a different physical key"
        );

        click_button("Save");
        assert_eq!(app.saved_hotkey(), expected, "{expected} must survive Save");
        app.land_fixture(
            bullet,
            expected,
            "macos_hotkey_native::records_the_spelled_combinations",
        );
    }
}

/// The modifier nibble `crate::hotkey` uses, for asserting on a recorded edge.
fn ui_modifiers(flags: CGEventFlags) -> u64 {
    let mut mask = 0u64;
    if flags.contains(CGEventFlags::CGEventFlagControl) {
        mask |= hotkey::MOD_CTRL as u64;
    }
    if flags.contains(CGEventFlags::CGEventFlagAlternate) {
        mask |= MOD_ALT as u64;
    }
    if flags.contains(CGEventFlags::CGEventFlagShift) {
        mask |= hotkey::MOD_SHIFT as u64;
    }
    if flags.contains(CGEventFlags::CGEventFlagCommand) {
        mask |= MOD_WIN as u64;
    }
    mask
}

/// `⌘C` records as `⌘C` and does not paste.
///
/// The second half needs a differential, not an absence: a copy with nothing
/// selected changes nothing either way, so "the interval field is untouched"
/// would be green with the swallow entirely broken. What is done instead is
/// the same keystroke twice — `⌘V` with a sentinel on the pasteboard, into the
/// same focused field — once idle and once armed. Idle it must paste; armed it
/// must not. That difference can only come from the monitor.
#[test]
#[ignore = "needs a real desktop; run via scripts/native-tests-macos.sh"]
fn cmd_c_records_and_does_not_reach_the_clipboard() {
    const SENTINEL: &str = "9137";
    let app = Sandboxed::settings("Ctrl+Shift+Space");
    set_pasteboard(SENTINEL);

    // Positive control: idle, the very same chord edits the window.
    focus_interval_field();
    press_chord(VK_ANSI_A, CGEventFlags::CGEventFlagCommand); // select all
    press_chord(VK_ANSI_V, CGEventFlags::CGEventFlagCommand);
    assert_eq!(
        Ui::settings().interval_field(),
        SENTINEL,
        "the control failed: ⌘V does not reach the interval field even when idle, \
         so the armed case below would prove nothing"
    );

    // Armed: the identical chord, the identical focused field, swallowed.
    arm();
    press_chord(VK_ANSI_V, CGEventFlags::CGEventFlagCommand);
    let ui = Ui::settings();
    assert_eq!(
        ui.interval_field(),
        SENTINEL,
        "⌘V edited the field while armed — the key reached the window as a command"
    );
    assert_eq!(
        ui.hotkey_field().as_deref(),
        Some("⌘V"),
        "⌘V was swallowed but not recorded: {:#?}",
        ui.0
    );

    // The bullet itself.
    arm();
    app.truncate_keystream();
    press_chord(VK_ANSI_C, CGEventFlags::CGEventFlagCommand);
    let ui = Ui::settings();
    assert_eq!(ui.hotkey_field().as_deref(), Some("⌘C"));
    assert_eq!(
        app.edges(),
        vec![(VK_ANSI_C as u64, MOD_WIN as u64)],
        "⌘C reached the monitor as something other than the C cap with ⌘ held"
    );
    assert_eq!(
        pasteboard(),
        SENTINEL,
        "the pasteboard changed — ⌘C was turned into a copy command"
    );
    click_button("Save");
    assert_eq!(app.saved_hotkey(), "⌘C");
    app.land_fixture(
        "cmd-c",
        "⌘C",
        "macos_hotkey_native::cmd_c_records_and_does_not_reach_the_clipboard",
    );
}

/// Keypad `5` is refused **by name**, and is not mistaken for the digit row.
///
/// The refusal text is read off the window, which is the only place "by name"
/// is observable. Stated so it cannot pass vacuously twice over: the hint must
/// name the keypad, and the recorder must still be armed — proved by the
/// *next* chord landing with no second Record click.
#[test]
#[ignore = "needs a real desktop; run via scripts/native-tests-macos.sh"]
fn the_keypad_is_refused_and_leaves_the_recorder_armed() {
    let app = Sandboxed::settings("Ctrl+Shift+Space");
    arm();
    press_chord(VK_ANSI_KEYPAD5, CGEventFlags::CGEventFlagCommand);

    let ui = Ui::settings();
    assert!(
        ui.hint().contains("Keypad 5"),
        "the refusal must name the key: {:?}",
        ui.hint()
    );
    assert!(ui.armed(), "a refusal must not disarm: {:#?}", ui.0);
    assert_eq!(
        app.edges(),
        vec![(VK_ANSI_KEYPAD5 as u64, MOD_WIN as u64)],
        "the keypad's 5 arrived as some other key code — the digit row's, perhaps"
    );
    app.land_fixture(
        "keypad-5",
        "⌘ + keypad 5",
        "macos_hotkey_native::the_keypad_is_refused_and_leaves_the_recorder_armed",
    );

    // Still armed: no second Record click.
    press("Alt+Cmd+J");
    click_button("Save");
    assert_eq!(
        app.saved_hotkey(),
        "⌥⌘J",
        "the keypad press either recorded as something, or disarmed the recorder"
    );
}

/// A bare letter is refused by name; a bare `F13` records. The F row types
/// nothing, which is what makes it the one exception to the modifier rule.
#[test]
#[ignore = "needs a real desktop; run via scripts/native-tests-macos.sh"]
fn a_bare_letter_is_refused_and_a_bare_f_key_records() {
    let refused = Sandboxed::settings("Ctrl+Shift+Space");
    arm();
    press_chord(VK_ANSI_J, CGEventFlags::CGEventFlagNull); // J, nothing held

    let ui = Ui::settings();
    assert!(
        ui.hint().contains('J'),
        "the refusal must name the key: {:?}",
        ui.hint()
    );
    assert!(ui.armed(), "a refusal must not disarm: {:#?}", ui.0);
    assert_eq!(refused.edges(), vec![(VK_ANSI_J as u64, 0)]);
    refused.land_fixture(
        "bare-letter",
        "J",
        "macos_hotkey_native::a_bare_letter_is_refused_and_a_bare_f_key_records",
    );
    press("Alt+Cmd+J"); // still armed
    click_button("Save");
    assert_eq!(refused.saved_hotkey(), "⌥⌘J");
    drop(refused);

    let recorded = Sandboxed::settings("Ctrl+Shift+Space");
    arm();
    press_chord(VK_F13, CGEventFlags::CGEventFlagNull);
    let ui = Ui::settings();
    assert_eq!(
        ui.hotkey_field().as_deref(),
        Some("F13"),
        "a bare F13 must record: {:#?}",
        ui.0
    );
    assert_eq!(recorded.edges(), vec![(VK_F13 as u64, 0)]);
    recorded.land_fixture(
        "bare-f13",
        "F13",
        "macos_hotkey_native::a_bare_letter_is_refused_and_a_bare_f_key_records",
    );
    click_button("Save");
    assert_eq!(recorded.saved_hotkey(), "F13");
}

/// `⌥⌘` alone changes nothing and shows no error, and neither does `fn`.
///
/// The recorded stream for these is **empty**, and an empty stream is what a
/// dead monitor produces too — so each null is preceded, in this same process,
/// by something that must land:
///
/// 1. `⌥⌘J` records. The monitor is installed, armed, and writing the stream.
/// 2. `⌥⌘J` with `fn` also held records, as plain `⌥⌘J`. That is the control
///    for the `fn` half specifically: it shows `CGEventFlagSecondaryFn` is
///    postable and does arrive, so an empty stream afterwards means `fn` alone
///    produced no press edge rather than that `fn` was never delivered.
///
/// Only then is the stream truncated and the modifiers held alone.
#[test]
#[ignore = "needs a real desktop; run via scripts/native-tests-macos.sh"]
fn modifiers_alone_and_fn_alone_are_not_events() {
    let app = Sandboxed::settings("Ctrl+Shift+Space");

    // Control 1 — the instrument is alive.
    arm();
    press_chord(VK_ANSI_J, alt_cmd());
    assert_eq!(
        app.edges(),
        vec![(VK_ANSI_J as u64, (MOD_ALT | MOD_WIN) as u64)],
        "the control chord never reached the monitor, so every null below is meaningless"
    );
    assert_eq!(Ui::settings().hotkey_field().as_deref(), Some("⌥⌘J"));

    // Control 2 — the fn flag is postable and does arrive, riding on a key
    // event. It is not one of the four modifiers, so it must not show up in
    // the nibble.
    arm();
    app.truncate_keystream();
    press_chord(VK_ANSI_J, alt_cmd() | CGEventFlags::CGEventFlagSecondaryFn);
    assert_eq!(
        app.edges(),
        vec![(VK_ANSI_J as u64, (MOD_ALT | MOD_WIN) as u64)],
        "fn either was not delivered, or was counted as a modifier"
    );

    // The nulls.
    arm();
    app.truncate_keystream();
    hold_modifiers_only(alt_cmd());
    let ui = Ui::settings();
    assert_eq!(app.edges(), vec![], "a modifier chord produced a press edge");
    assert!(ui.armed(), "holding ⌥⌘ ended the recording: {:#?}", ui.0);
    assert_eq!(
        ui.hint(),
        PROMPT,
        "holding ⌥⌘ showed an error instead of nothing"
    );
    app.land_fixture(
        "modifiers-only",
        "⌥⌘ held, nothing else (after ⌥⌘J proved the monitor live)",
        "macos_hotkey_native::modifiers_alone_and_fn_alone_are_not_events",
    );

    app.truncate_keystream();
    hold_modifiers_only(CGEventFlags::CGEventFlagSecondaryFn);
    let ui = Ui::settings();
    assert_eq!(app.edges(), vec![], "fn produced a press edge");
    assert!(ui.armed(), "holding fn ended the recording: {:#?}", ui.0);
    assert_eq!(ui.hint(), PROMPT, "holding fn showed an error");
    app.land_fixture(
        "fn-only",
        "fn held, nothing else (after ⌥⌘fnJ proved fn is delivered)",
        "macos_hotkey_native::modifiers_alone_and_fn_alone_are_not_events",
    );

    click_button("Cancel");
    click_button("Save");
    assert_eq!(
        app.saved_hotkey(),
        "⌃⇧Space",
        "nothing was pressed, so nothing may have changed"
    );
}

/// With a Russian layout active, the cap that types `о` still records `⌥⌘J`.
/// A key is a position, not the character it produces.
///
/// The layout is switched through `TISSelectInputSource` and then **read
/// back**, so the precondition is observed rather than assumed; the value read
/// is what goes into the fixture's provenance line. A guard puts the previous
/// source back — this guest is shared, and a layout left switched is a wrong
/// answer owed to every later test.
#[test]
#[ignore = "needs a real desktop; run via scripts/native-tests-macos.sh"]
fn a_russian_layout_records_the_same_physical_key() {
    let _layout = LayoutGuard::switch_to("com.apple.keylayout.Russian");

    let app = Sandboxed::settings("Ctrl+Shift+Space");
    arm();
    press_chord(VK_ANSI_J, alt_cmd());

    let ui = Ui::settings();
    assert_eq!(
        ui.hotkey_field().as_deref(),
        Some("⌥⌘J"),
        "the layout moved the shortcut to a different key: {:#?}",
        ui.0
    );
    assert_eq!(
        app.edges(),
        vec![(VK_ANSI_J as u64, (MOD_ALT | MOD_WIN) as u64)],
        "the key code changed with the layout — it is supposed to be a position"
    );
    click_button("Save");
    assert_eq!(app.saved_hotkey(), "⌥⌘J");
    app.land_fixture(
        "russian-layout-alt-cmd-j",
        "⌥⌘ + the cap that types о",
        "macos_hotkey_native::a_russian_layout_records_the_same_physical_key",
    );
}

/// Bare `Esc` puts the saved hotkey back, and so does losing the focus —
/// whether the focus is taken by another app or by `⌘Tab` itself.
///
/// Non-vacuous by construction: after the cancelling input, a perfectly good
/// chord is pressed **without re-arming**. If it lands, the recorder never
/// cancelled — and that is the difference between "cancelled" and "captured
/// something that happened to equal the saved value".
#[test]
#[ignore = "needs a real desktop; run via scripts/native-tests-macos.sh"]
fn escape_and_focus_loss_put_the_saved_hotkey_back() {
    // Esc. Pressed as a `kVK_*` literal because §5 refuses a bare Esc as a
    // hotkey, so it cannot be addressed through the parser.
    let app = Sandboxed::settings("Alt+Cmd+J");
    arm();
    press_chord(VK_ESCAPE, CGEventFlags::CGEventFlagNull);
    let ui = Ui::settings();
    assert!(!ui.armed(), "Esc did not end the recording: {:#?}", ui.0);
    assert_eq!(ui.hotkey_field().as_deref(), Some("⌥⌘J"));
    assert_eq!(
        app.edges(),
        vec![(VK_ESCAPE as u64, 0)],
        "the Esc that cancelled was not the Esc cap"
    );
    app.land_fixture(
        "bare-escape",
        "Esc",
        "macos_hotkey_native::escape_and_focus_loss_put_the_saved_hotkey_back",
    );
    press("Ctrl+Shift+Space"); // must be ignored: the recorder is idle
    click_button("Save");
    assert_eq!(
        app.saved_hotkey(),
        "⌥⌘J",
        "Esc did not cancel — a later chord still recorded"
    );
    drop(app);

    // Another app taking the focus.
    let app = Sandboxed::settings("Alt+Cmd+J");
    arm();
    steal_focus();
    press("Ctrl+Shift+Space");
    let ui = Ui::settings();
    assert!(
        !ui.armed(),
        "focus loss did not end the recording: {:#?}",
        ui.0
    );
    assert_eq!(ui.hotkey_field().as_deref(), Some("⌥⌘J"));
    click_button("Save");
    assert_eq!(
        app.saved_hotkey(),
        "⌥⌘J",
        "focus loss did not cancel — a later chord still recorded"
    );
    drop(app);

    // ⌘Tab itself. macOS consumes the chord for the switcher, so what the
    // recorder can observe is only the focus going away — the same thing the
    // case above stages by other means. Posting the literal chord is what
    // shows the two are in fact the same, rather than asserting it.
    // Bound rather than dropped: the sandbox owns the HOME the window is
    // reading, and this block asserts on the window rather than on the file.
    let _app = Sandboxed::settings("Alt+Cmd+J");
    arm();
    press_chord(VK_TAB, CGEventFlags::CGEventFlagCommand);
    press("Ctrl+Shift+Space"); // must be ignored: the recorder cancelled
    let ui = Ui::settings();
    assert!(!ui.armed(), "⌘Tab did not end the recording: {:#?}", ui.0);
    assert_eq!(
        ui.hotkey_field().as_deref(),
        Some("⌥⌘J"),
        "⌘Tab did not cancel — a later chord still recorded"
    );
    // The liveness control, and the whole reason this block is not a pair of
    // vacuous negatives: "no chord landed" is also what a switcher left on
    // screen produces, eating every key posted after it. So the window is
    // armed again and a *different* combination has to land.
    arm();
    press("Ctrl+Shift+Space");
    let ui = Ui::settings();
    assert!(
        !ui.armed(),
        "after ⌘Tab the window stopped taking keys at all, so the negative \
         above proves nothing: {:#?}",
        ui.0
    );
    assert_eq!(
        ui.hotkey_field().as_deref(),
        Some("⌃⇧Space"),
        "a fresh recording after ⌘Tab did not land"
    );
}

/// Pressing the app's own current hotkey opens the popup and ends the
/// recording cancelled. The running instance holds it system-wide while the
/// settings window is a separate process, so the Carbon binding wins and the
/// settings monitor never sees the key.
///
/// The settings side gets its own positive control: a chord the main instance
/// does **not** hold must land in the settings stream, so "the monitor saw
/// nothing" is distinguishable from "the monitor is nothing".
#[test]
#[ignore = "needs a real desktop; run via scripts/native-tests-macos.sh"]
fn the_apps_own_hotkey_opens_the_popup_instead_of_recording() {
    // The settings window goes up first, alone, and its control runs while it
    // is the only `nudge` on the machine. That ordering is not cosmetic:
    // measured in this guest, a settings window that has never been the key
    // window publishes no accessibility tree at all, and starting the main
    // instance first puts its always-on-top popup over this one before it ever
    // gets a frame. Which process was launched first is no part of the claim
    // being tested; both are alive for the part that is.
    let settings = Sandboxed::settings("Alt+Cmd+J");

    // Control: the settings monitor works, for a key nobody has bound.
    arm();
    press_chord(VK_F13, CGEventFlags::CGEventFlagNull);
    assert_eq!(
        settings.edges(),
        vec![(VK_F13 as u64, 0)],
        "the settings monitor records nothing at all, so the null below proves nothing"
    );

    let main = Sandboxed::main_instance("Alt+Cmd+J");
    // Its startup popup opens on top and takes the focus, so the Esc lands
    // there rather than on the settings window.
    quiesce_popup(main.pid());

    arm();
    settings.truncate_keystream();
    press("Alt+Cmd+J");
    assert!(
        popup_visible(main.pid()),
        "the main instance did not get its own hotkey"
    );
    assert_eq!(
        settings.edges(),
        vec![],
        "the settings monitor saw a key the main process had bound"
    );

    // The popup taking the focus is what cancels the recording; proved
    // non-vacuously by the next chord not landing.
    press("Ctrl+Shift+Space");
    click_button("Save");
    assert_eq!(
        settings.saved_hotkey(),
        "⌥⌘J",
        "the recording did not end when the popup took the focus"
    );
}

/// `⌘Space` is reported as taken by the system rather than accepted silently.
///
/// The two answers disagreeing is the fact being pinned, not a bug: the
/// registration call reports nothing about another holder — that is the
/// measurement this whole change rests on — and the system's own table is the
/// only source that can see Spotlight.
#[test]
#[ignore = "reads the live system shortcut table and registers a real hotkey"]
fn cmd_space_registers_and_is_still_reported_as_the_systems() {
    let hk = hotkey::parse("Cmd+Space").expect("parses");
    let registration = hotkey_carbon::register(&hk);
    assert!(
        registration.is_ok(),
        "registration must succeed — it says nothing about who else holds it"
    );
    drop(registration);
    assert!(
        matches!(
            hotkey_carbon::assess(&hk),
            hotkey_carbon::HotkeyStatus::HeldBySystem(_)
        ),
        "⌘Space must be reported as the system's, from the system's own table"
    );
}

/// ...and the user is the one told. The status above is only evidence for the
/// bullet if it reaches the window, which is the half nothing else covers:
/// `Save` turns it into a banner, and the banner is read back off the window.
#[test]
#[ignore = "needs a real desktop; run via scripts/native-tests-macos.sh"]
fn saving_cmd_space_says_the_system_holds_it() {
    let app = Sandboxed::settings("Cmd+Space");
    assert_eq!(Ui::settings().hotkey_field().as_deref(), Some("⌘Space"));

    click_button("Save");
    let banner = Ui::settings()
        .banner()
        .expect("Save must say something");
    assert!(
        banner.contains("system shortcut"),
        "⌘Space was accepted silently: {banner:?}"
    );
    assert_eq!(app.saved_hotkey(), "⌘Space", "it is a warning, not a refusal");
    // The window is addressed by content, not by pid, so a second settings
    // window would make every reading below ambiguous.
    drop(app);

    // The control: an unclaimed combination saves with no such warning, so the
    // assertion above is about ⌘Space rather than about every Save.
    let free = Sandboxed::settings("Alt+Cmd+J");
    click_button("Save");
    let banner = Ui::settings().banner().expect("Save must say something");
    assert_eq!(banner, "Saved", "⌥⌘J is not the system's: {banner:?}");
    assert_eq!(free.saved_hotkey(), "⌥⌘J");
}

/// What actually happens when `⌘Space` is pressed at an armed recorder, which
/// nothing else in this suite finds out. Spotlight may take the chord before
/// the app-local monitor ever sees it — the point of the assertion is that
/// both outcomes are *coherent*: either the recorder captured exactly `⌘Space`
/// and stopped, or it saw nothing and is still waiting. A field holding
/// something else, or a row that disarmed with nothing recorded, is a bug.
#[test]
#[ignore = "needs a real desktop; run via scripts/native-tests-macos.sh"]
fn pressing_cmd_space_while_armed_either_records_it_or_never_arrives() {
    let app = Sandboxed::settings("Ctrl+Shift+Space");
    arm();
    press_chord(VK_SPACE, CGEventFlags::CGEventFlagCommand);

    let edges = app.edges();
    // Whatever Spotlight did with the chord, get the desktop back before the
    // next test runs: this guest is shared and a search field left open eats
    // the following keystrokes.
    press_chord(VK_ESCAPE, CGEventFlags::CGEventFlagNull);

    let ui = Ui::settings();
    match edges.as_slice() {
        [] => {
            eprintln!("[observed] ⌘Space never reached the app-local monitor");
            // Either nothing happened at all, or Spotlight took the focus and
            // the recording cancelled. Both are coherent; what may not happen
            // is the field acquiring a combination no edge delivered.
            if let Some(field) = ui.hotkey_field() {
                assert_eq!(
                    field, "⌃⇧Space",
                    "the field changed with nothing recorded: {:#?}",
                    ui.0
                );
            } else {
                assert_eq!(ui.hint(), PROMPT, "still armed, but reporting something");
            }
        }
        [(code, mods)] => {
            eprintln!("[observed] ⌘Space reached the monitor as {code}/{mods}");
            assert_eq!((*code, *mods), (VK_SPACE as u64, MOD_WIN as u64));
            assert_eq!(
                ui.hotkey_field().as_deref(),
                Some("⌘Space"),
                "⌘Space was recorded but the field does not show it: {:#?}",
                ui.0
            );
        }
        other => panic!("one press produced {} edges: {other:?}", other.len()),
    }
}

/// The captured value survives `Save` and a relaunch, and the app logs no
/// error.
///
/// "Survives" is taken to mean the value still *works*, not only that the same
/// bytes read back: a main instance is started on the same HOME afterwards and
/// the captured combination has to make its popup appear. And the logging half
/// is asserted rather than assumed — the child's stderr is captured and has to
/// carry no error line.
#[test]
#[ignore = "needs a real desktop; run via scripts/native-tests-macos.sh"]
fn a_captured_hotkey_survives_save_and_a_relaunch() {
    let mut app = Sandboxed::settings_logging("Ctrl+Shift+Space");
    arm();
    press_chord(VK_ANSI_J, alt_cmd());
    click_button("Save");
    assert_eq!(app.saved_hotkey(), "⌥⌘J");

    // Everything the app reports goes to stderr behind a `[nudge…]` prefix, so
    // the absence of an error is checkable rather than a figure of speech.
    let log = app.stderr_after_exit();
    let errors: Vec<&str> = log
        .lines()
        .filter(|l| l.contains("[nudge"))
        .filter(|l| {
            ["cannot", "could not", "failed", "unavailable", "error"]
                .iter()
                .any(|w| l.to_ascii_lowercase().contains(w))
        })
        .collect();
    assert!(
        errors.is_empty(),
        "the recording logged errors:\n{}",
        errors.join("\n")
    );

    app.relaunch_settings();
    let ui = Ui::settings();
    assert_eq!(
        ui.hotkey_field().as_deref(),
        Some("⌥⌘J"),
        "the relaunched window does not show the captured value: {:#?}",
        ui.0
    );
    assert!(
        window_visible(app.pid()),
        "the relaunched settings window never opened"
    );

    // The value is not only stored, it binds: a main instance on the same HOME
    // must answer the captured combination. The settings window goes first —
    // the startup popup has to be dismissed with an Esc, and a settings window
    // is what would otherwise take it.
    app.kill_child();
    let main = app.spawn_main_on_same_home();
    quiesce_popup(main.pid());
    press("Alt+Cmd+J");
    assert!(
        popup_visible(main.pid()),
        "the captured hotkey was saved but never registered"
    );
}
