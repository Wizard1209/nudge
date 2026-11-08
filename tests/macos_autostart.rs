//! The macOS LaunchAgent provider against a real filesystem.
//!
//! # Why the two halves are tested differently
//!
//! `MacLaunchAgentProvider` does two things, and only one of them can be
//! sandboxed:
//!
//!   * it writes `~/Library/LaunchAgents/com.wizard1209.nudge.plist`. The
//!     provider is built over an explicit home (`MacLaunchAgentProvider::in_home`),
//!     so every test here builds one over a fresh temp dir and the whole file
//!     footprint lands there — the developer's real `~/Library/LaunchAgents`
//!     is never touched, and no test changes the process's `HOME`.
//!
//!   * it registers the job with `launchctl`. The home has no effect on that:
//!     launchd's per-user domain is keyed by uid, so there is no sandboxed
//!     domain to bootstrap into and every `launchctl` call would hit the
//!     developer's live login session. Every test here that would invoke
//!     `launchctl` — which includes `disable()`, not just `enable()` — is
//!     therefore `#[ignore]`d, and the launchctl path is covered by
//!     `nudge --autostart-selftest` instead: a deliberate manual act on the
//!     real machine. Nothing that runs under a plain `cargo test` shells out
//!     to `launchctl` at all.

#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};

use nudge_web::autostart::{
    AutostartProvider, LAUNCH_AGENT_LABEL, MacLaunchAgentProvider, apply_autostart,
    launch_agent_plist_path, parse_plist_label,
};
use nudge_web::config::Config;

/// Everything a sandboxed test holds: the temp home (deleted on drop), the
/// plist path a provider over it must derive, and that provider.
struct Sandbox {
    home: tempfile::TempDir,
    plist: PathBuf,
    provider: MacLaunchAgentProvider,
}

/// A provider over a fresh temp home, naming this test binary as the program.
fn sandbox() -> Sandbox {
    let home = tempfile::tempdir().expect("temp HOME");
    let plist = launch_agent_plist_path(home.path(), LAUNCH_AGENT_LABEL);
    let program = std::env::current_exe().expect("current_exe");
    let provider = MacLaunchAgentProvider::in_home(home.path(), program);
    Sandbox {
        home,
        plist,
        provider,
    }
}

/// The precondition every other test in this file rests on: a provider built
/// over a home derives its plist path under that home. If this ever goes red,
/// the rest of the file is writing somewhere other than its sandbox.
#[test]
fn a_provider_over_a_home_keeps_its_plist_under_it() {
    let sb = sandbox();

    assert_eq!(sb.provider.plist_path(), sb.plist.as_path());
    assert!(
        sb.provider.plist_path().starts_with(sb.home.path()),
        "plist path {} escaped the sandbox",
        sb.provider.plist_path().display()
    );
}

/// The file half of the enable/disable lifecycle, end to end, against a real
/// filesystem: nothing → plist present and recognised as ours → nothing.
#[test]
fn plist_is_written_confirmed_and_removed() {
    let sb = sandbox();
    let plist = &sb.plist;
    let provider = &sb.provider;

    assert!(!plist.exists());
    assert!(
        !provider.is_enabled().unwrap(),
        "clean HOME starts disabled"
    );

    provider.write_plist().expect("write plist");

    assert!(plist.exists(), "{} was not created", plist.display());
    let xml = std::fs::read_to_string(plist).expect("read plist");
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
    let sb = sandbox();
    let plist = &sb.plist;
    let provider = &sb.provider;

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
    let sb = sandbox();
    let plist = &sb.plist;
    std::fs::create_dir_all(plist.parent().unwrap()).unwrap();
    std::fs::write(
        plist,
        nudge_web::autostart::launch_agent_plist_xml("com.example.other", Path::new("/bin/true")),
    )
    .unwrap();

    assert!(
        !sb.provider.is_enabled().unwrap(),
        "a plist labelled com.example.other is not our agent"
    );
}

/// The `launchctl` half, plus `apply_autostart` over the real provider.
/// Ignored by default and it must stay that way, for two independent reasons:
///
///   * `enable()` bootstraps a real job into the developer's live login
///     session — a sandboxed home cannot redirect launchd's per-user domain,
///     which is keyed by uid — and `RunAtLoad` makes launchd execute the
///     program named in the plist the moment it loads. Here that program is
///     the *test binary*.
///   * `disable()` boots the label out of that same live domain, which
///     terminates a Nudge the developer actually has running.
///
/// Neither belongs in a `cargo test` that someone runs while working.
/// `nudge --autostart-selftest` is the supported way to exercise this path; it
/// restores whatever state it found.
#[test]
#[ignore = "bootstraps into the live launchd gui/<uid> domain (a sandboxed home cannot redirect it), \
            RunAtLoad makes launchd exec the program, and bootout would kill a running \
            Nudge; use `nudge --autostart-selftest` instead"]
fn launchctl_registration_round_trip() {
    let sb = sandbox();
    let plist = &sb.plist;
    let provider = &sb.provider;

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
    apply_autostart(provider, &mut config, true, |c| {
        *saved.borrow_mut() = Some(c.clone());
        Ok(())
    })
    .expect("enable through apply_autostart");
    assert!(plist.exists(), "apply_autostart(true) left no plist");
    assert!(config.autostart);
    assert_eq!(saved.borrow().as_ref().map(|c| c.autostart), Some(true));

    apply_autostart(provider, &mut config, false, |c| {
        *saved.borrow_mut() = Some(c.clone());
        Ok(())
    })
    .expect("disable through apply_autostart");
    assert!(!plist.exists());
    assert!(!config.autostart);
}
