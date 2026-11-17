#![cfg(target_os = "macos")]

use std::path::{Path, PathBuf};

use nudge_web::autostart::{
    AutostartProvider, LAUNCH_AGENT_LABEL, MacLaunchAgentProvider, apply_autostart,
    launch_agent_plist_path, parse_plist_label,
};
use nudge_web::config::Config;

struct Sandbox {
    home: tempfile::TempDir,
    plist: PathBuf,
    provider: MacLaunchAgentProvider,
}

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
    provider.enable().expect("enable twice");
    assert!(provider.is_enabled().unwrap());

    provider.disable().expect("disable");
    assert!(!plist.exists());
    assert!(!provider.is_enabled().unwrap());
    provider.disable().expect("disable twice");

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
