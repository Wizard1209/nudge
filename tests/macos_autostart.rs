//! The macOS LaunchAgent provider against a real filesystem.
//!
//! # Why the two halves are tested differently
//!
//! `MacLaunchAgentProvider` does two things, and only one of them can be
//! sandboxed:
//!
//!   * it writes `~/Library/LaunchAgents/com.wizard1209.nudge.plist`. That
//!     path is derived through `dirs::home_dir()`, which on macOS reads `$HOME`
//!     first — verified by observation, not assumed:
//!     `probe_home_override_is_honoured` is that observation, and it fails
//!     loudly rather than letting the rest of the file quietly write into the
//!     developer's real `~/Library/LaunchAgents`. Overriding `HOME` therefore
//!     moves the whole footprint into a temp dir, exactly as
//!     `tests/macos_native.rs` does for the config and journal.
//!
//!   * it registers the job with `launchctl`. `HOME` has no effect on that:
//!     launchd's per-user domain is keyed by uid, so there is no sandboxed
//!     domain to bootstrap into and every `launchctl` call would hit the
//!     developer's live login session. Every test here that would invoke
//!     `launchctl` — which includes `disable()`, not just `enable()` — is
//!     therefore `#[ignore]`d, and the launchctl path is covered by
//!     `nudge --autostart-selftest` instead: a deliberate manual act on the
//!     real machine. Nothing that runs under a plain `cargo test` shells out
//!     to `launchctl` at all.
//!
//! # `HOME` is process-global
//!
//! Cargo runs the tests in one binary on threads, so `set_var("HOME", …)`
//! from one test is visible to all of them — and the provider reads `HOME`
//! when it is *constructed*, not when the file is written. Under default
//! parallelism that shows up as tests writing their plist into a sibling
//! test's temp dir and then failing to find it. `HOME_LOCK` serialises the
//! whole set-then-use region, so plain `cargo test` (what CI runs) is correct
//! without `--test-threads=1`.

#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use nudge_web::autostart::{
    AutostartProvider, LAUNCH_AGENT_LABEL, MacLaunchAgentProvider, apply_autostart,
    launch_agent_plist_path, parse_plist_label,
};
use nudge_web::config::Config;

/// Serialises every test that touches `HOME`. See the module header.
static HOME_LOCK: Mutex<()> = Mutex::new(());

/// Everything a sandboxed test holds: the lock (for as long as `HOME` is
/// redirected), the temp dir (deleted on drop), and the plist path the
/// provider is now expected to derive.
struct Sandbox {
    _lock: MutexGuard<'static, ()>,
    home: tempfile::TempDir,
    plist: PathBuf,
}

/// Point `HOME` at a fresh temp dir, under the lock.
///
/// `set_var` is `unsafe` in edition 2024 precisely because it is not
/// thread-safe; `HOME_LOCK` is what makes it safe here. A failing test
/// poisons the lock, and recovering the guard is right — the invariant being
/// protected is "one test owns `HOME` at a time", which a panic does not
/// break.
fn sandboxed_home() -> Sandbox {
    let lock = HOME_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let home = tempfile::tempdir().expect("temp HOME");
    unsafe { std::env::set_var("HOME", home.path()) };
    let plist = launch_agent_plist_path(home.path(), LAUNCH_AGENT_LABEL);
    Sandbox {
        _lock: lock,
        home,
        plist,
    }
}

/// The precondition every other test in this file rests on: `dirs` — and
/// therefore the provider's path derivation — actually follows an overridden
/// `HOME` on macOS. If this ever goes red, the rest of the file is writing
/// into the developer's real `~/Library/LaunchAgents` and must not run.
#[test]
fn probe_home_override_is_honoured() {
    let sb = sandboxed_home();

    assert_eq!(
        dirs::home_dir().as_deref(),
        Some(sb.home.path()),
        "dirs::home_dir() ignored the overridden HOME — every other test here \
         would write into the real ~/Library/LaunchAgents"
    );

    let provider = MacLaunchAgentProvider::for_current_exe().expect("provider");
    assert_eq!(provider.plist_path(), sb.plist.as_path());
    assert!(
        provider.plist_path().starts_with(sb.home.path()),
        "plist path {} escaped the sandbox",
        provider.plist_path().display()
    );
}

/// The file half of the enable/disable lifecycle, end to end, against a real
/// filesystem: nothing → plist present and recognised as ours → nothing.
#[test]
fn plist_is_written_confirmed_and_removed() {
    let sb = sandboxed_home();
    let plist = &sb.plist;
    let provider = MacLaunchAgentProvider::for_current_exe().expect("provider");

    assert!(!plist.exists());
    assert!(
        !provider.is_enabled().unwrap(),
        "clean HOME starts disabled"
    );

    provider.write_plist().expect("write plist");

    assert!(plist.exists(), "{} was not created", plist.display());
    let xml = std::fs::read_to_string(&plist).expect("read plist");
    assert_eq!(parse_plist_label(&xml).as_deref(), Some(LAUNCH_AGENT_LABEL));
    assert!(xml.contains("<key>RunAtLoad</key>"));
    assert!(
        xml.contains(&format!(
            "<string>{}</string>",
            provider.program().display()
        )),
        "plist does not name the program path {}:\n{xml}",
        provider.program().display()
    );
    assert!(
        provider.is_enabled().unwrap(),
        "is_enabled must see the file it just wrote — apply_autostart uses it to confirm"
    );

    assert!(provider.remove_plist().unwrap(), "removed an existing file");
    assert!(!plist.exists());
    assert!(!provider.is_enabled().unwrap());
}

/// Removing a plist that is not there is success, not an error — the
/// post-condition "no launch agent" already holds.
#[test]
fn removing_an_absent_plist_is_success() {
    let sb = sandboxed_home();
    let plist = &sb.plist;
    let provider = MacLaunchAgentProvider::for_current_exe().expect("provider");

    assert!(!plist.exists());
    assert!(
        !provider.remove_plist().unwrap(),
        "nothing was there to remove"
    );
    assert!(provider.remove_plist().is_ok(), "and it stays idempotent");
}

/// A foreign file squatting on our filename must not be reported as our
/// registration — otherwise `disable` would delete someone else's agent and
/// `is_enabled` would confirm a state we never wrote.
#[test]
fn a_foreign_plist_at_our_path_is_not_reported_as_enabled() {
    let sb = sandboxed_home();
    let plist = &sb.plist;
    std::fs::create_dir_all(plist.parent().unwrap()).unwrap();
    std::fs::write(
        &plist,
        nudge_web::autostart::launch_agent_plist_xml("com.example.other", Path::new("/bin/true")),
    )
    .unwrap();

    let provider = MacLaunchAgentProvider::for_current_exe().expect("provider");
    assert!(
        !provider.is_enabled().unwrap(),
        "a plist labelled com.example.other is not our agent"
    );
}

/// The `launchctl` half, plus `apply_autostart` over the real provider.
/// Ignored by default and it must stay that way, for two independent reasons:
///
///   * `enable()` bootstraps a real job into the developer's live login
///     session — `HOME` cannot redirect launchd's per-user domain, which is
///     keyed by uid — and `RunAtLoad` makes launchd execute the program named
///     in the plist the moment it loads. Under a redirected `HOME` that
///     program is the *test binary*.
///   * `disable()` boots the label out of that same live domain, which
///     terminates a Nudge the developer actually has running.
///
/// Neither belongs in a `cargo test` that someone runs while working.
/// `nudge --autostart-selftest` is the supported way to exercise this path; it
/// restores whatever state it found.
#[test]
#[ignore = "bootstraps into the live launchd gui/<uid> domain (HOME cannot redirect it), \
            RunAtLoad makes launchd exec the program, and bootout would kill a running \
            Nudge; use `nudge --autostart-selftest` instead"]
fn launchctl_registration_round_trip() {
    let sb = sandboxed_home();
    let plist = &sb.plist;
    let provider = MacLaunchAgentProvider::for_current_exe().expect("provider");

    let initial = provider.is_enabled().expect("is_enabled");
    assert!(!initial, "test HOME should have started clean");

    provider.enable().expect("enable");
    assert!(plist.exists());
    assert!(provider.is_enabled().unwrap());
    // Idempotent: the second enable re-registers rather than tripping over
    // the label it already installed.
    provider.enable().expect("enable twice");
    assert!(provider.is_enabled().unwrap());

    provider.disable().expect("disable");
    assert!(!plist.exists());
    assert!(!provider.is_enabled().unwrap());
    provider.disable().expect("disable twice");

    // And the transactional wrapper on top: config.autostart must only ever
    // claim a state the OS actually has.
    let mut config = Config::default();
    let saved = std::cell::RefCell::new(None::<Config>);
    apply_autostart(&provider, &mut config, true, |c| {
        *saved.borrow_mut() = Some(c.clone());
        Ok(())
    })
    .expect("enable through apply_autostart");
    assert!(plist.exists(), "apply_autostart(true) left no plist");
    assert!(config.autostart);
    assert_eq!(saved.borrow().as_ref().map(|c| c.autostart), Some(true));

    apply_autostart(&provider, &mut config, false, |c| {
        *saved.borrow_mut() = Some(c.clone());
        Ok(())
    })
    .expect("disable through apply_autostart");
    assert!(!plist.exists());
    assert!(!config.autostart);
}
