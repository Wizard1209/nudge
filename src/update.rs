//! The Settings window's update check (spec §9 "Updates").
//!
//! - **The pure half** — version order, which asset is the Windows download,
//!   what a `digest` says, the replace sequence on a directory, and the row's
//!   table ([`step`]). No network; unit-tested here.
//! - **The network half** — [`fetch_latest`] and [`download`], the only two
//!   requests the app ever makes, each run on a worker thread when the row's
//!   button is pressed.
//!
//! Not compiled for wasm: the browser build has no Updates group.

use std::cmp::Ordering;
use std::path::{Path, PathBuf};

/// Where the check looks, as the row's footnote names it.
pub const REPO: &str = "github.com/Wizard1209/nudge";

/// The one request the check makes.
pub const LATEST_URL: &str = "https://api.github.com/repos/Wizard1209/nudge/releases/latest";

/// The footnote under the Updates group: where it looks, and when.
pub const FOOTNOTE: &str =
    "Checks github.com/Wizard1209/nudge for new releases, only when pressed.";

/// The running version, as the row shows it and the check compares against.
pub const CURRENT: &str = env!("CARGO_PKG_VERSION");

/// The fields of GitHub's release object the check reads.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct Release {
    pub tag_name: String,
    pub html_url: String,
    #[serde(default)]
    pub assets: Vec<Asset>,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct Asset {
    pub name: String,
    /// `sha256:<hex>`; absent on a release GitHub has not hashed.
    #[serde(default)]
    pub digest: Option<String>,
    pub browser_download_url: String,
}

/// The release object in GitHub's reply, or the reason the row shows.
pub fn parse_release(body: &str) -> Result<Release, String> {
    serde_json::from_str(body).map_err(|_| UNREADABLE.to_string())
}

const UNREADABLE: &str = "GitHub sent a reply Nudge cannot read";
const OFFLINE: &str = "Offline: could not reach GitHub";

/// A version by semver's precedence rules: the `X.Y.Z` core, then its
/// pre-release identifiers. Build metadata is dropped, as precedence does.
#[derive(Debug, PartialEq, Eq)]
struct Semver<'a> {
    core: [u64; 3],
    pre: Vec<&'a str>,
}

fn parse_version(text: &str) -> Option<Semver<'_>> {
    let text = version_of(text);
    let text = text.split_once('+').map_or(text, |(v, _)| v);
    let (core, pre) = match text.split_once('-') {
        Some((core, pre)) => (core, pre.split('.').collect()),
        None => (text, Vec::new()),
    };
    let mut parts = core.split('.');
    let mut number = || {
        let p = parts.next()?;
        (!p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
            .then(|| p.parse().ok())
            .flatten()
    };
    let core = [number()?, number()?, number()?];
    let well_formed =
        |id: &&str| !id.is_empty() && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-');
    (parts.next().is_none() && pre.iter().all(well_formed)).then_some(Semver { core, pre })
}

impl Ord for Semver<'_> {
    fn cmp(&self, other: &Self) -> Ordering {
        self.core.cmp(&other.core).then_with(|| {
            match (self.pre.is_empty(), other.pre.is_empty()) {
                // A release outranks every pre-release of its own core.
                (true, true) => Ordering::Equal,
                (true, false) => Ordering::Greater,
                (false, true) => Ordering::Less,
                (false, false) => {
                    for (a, b) in self.pre.iter().zip(&other.pre) {
                        let order = match (a.parse::<u64>(), b.parse::<u64>()) {
                            (Ok(a), Ok(b)) => a.cmp(&b),
                            // Numeric identifiers rank below alphanumeric ones.
                            (Ok(_), Err(_)) => Ordering::Less,
                            (Err(_), Ok(_)) => Ordering::Greater,
                            (Err(_), Err(_)) => a.cmp(b),
                        };
                        if order != Ordering::Equal {
                            return order;
                        }
                    }
                    self.pre.len().cmp(&other.pre.len())
                }
            }
        })
    }
}

impl PartialOrd for Semver<'_> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Whether `tag` names a version newer than `current` — semver precedence,
/// `v` stripped. An older or equal latest, or a tag that is not a version,
/// is not an update.
pub fn is_newer(tag: &str, current: &str) -> bool {
    match (parse_version(tag), parse_version(current)) {
        (Some(tag), Some(current)) => tag > current,
        _ => false,
    }
}

/// The version a tag names, as the row shows it: `v0.3.0` → `0.3.0`.
pub fn version_of(tag: &str) -> &str {
    let tag = tag.trim();
    tag.strip_prefix('v').unwrap_or(tag)
}

/// What `Update` fetches, and what it must hash to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Download {
    pub url: String,
    pub sha256: [u8; 32],
}

/// The release's Windows exe — by the name the release workflow gives it —
/// and its SHA-256, or the reason the update is refused. A missing or
/// unreadable digest refuses: the exe that replaces the running one is
/// installed only when it can be checked.
pub fn windows_download(release: &Release) -> Result<Download, String> {
    let name = format!(
        "nudge-v{}-x86_64-windows.exe",
        version_of(&release.tag_name)
    );
    let asset = release
        .assets
        .iter()
        .find(|a| a.name == name)
        .ok_or("The release has no Windows download")?;
    let sha256 = asset
        .digest
        .as_deref()
        .and_then(parse_digest)
        .ok_or("No checksum for the Windows download")?;
    Ok(Download {
        url: asset.browser_download_url.clone(),
        sha256,
    })
}

/// `sha256:<64 hex digits>`, in either case.
fn parse_digest(digest: &str) -> Option<[u8; 32]> {
    let hex = digest.strip_prefix("sha256:")?;
    if hex.len() != 64 || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [0u8; 32];
    for (i, byte) in out.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// `Ok` when `bytes` hash to `sha256`.
pub fn verify(bytes: &[u8], sha256: &[u8; 32]) -> Result<(), String> {
    let actual = ring::digest::digest(&ring::digest::SHA256, bytes);
    if actual.as_ref() == sha256 {
        Ok(())
    } else {
        Err("Download does not match its checksum".into())
    }
}

/// Where the replaced exe is kept until the new one starts: `<exe>.old`.
pub fn old_path(exe: &Path) -> PathBuf {
    with_suffix(exe, ".old")
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    name.into()
}

/// Put `bytes` at `exe`'s path, keeping the running exe as `<exe>.old`.
///
/// A running exe can be renamed but not overwritten, so: write the new one
/// beside it, rename the running one aside, rename the new one into place.
/// A failure of that last rename puts the old one back, so `exe` is never
/// left missing.
pub fn install_over(exe: &Path, bytes: &[u8]) -> Result<(), String> {
    let failed = |e: std::io::Error| format!("Could not replace Nudge: {e}");
    let new = with_suffix(exe, ".new");
    let old = old_path(exe);
    let write = || -> std::io::Result<()> {
        use std::io::Write;
        let mut file = std::fs::File::create(&new)?;
        file.write_all(bytes)?;
        file.sync_all()
    };
    let swap = || -> std::io::Result<()> {
        // What an earlier update set aside, if its app never got to delete it.
        match std::fs::remove_file(&old) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
        std::fs::rename(exe, &old)?;
        std::fs::rename(&new, exe).inspect_err(|_| {
            let _ = std::fs::rename(&old, exe);
        })
    };
    write().and_then(|()| swap()).map_err(|e| {
        let _ = std::fs::remove_file(&new);
        failed(e)
    })
}

/// Delete what [`install_over`] set aside, if it is there. `Ok(false)` when
/// there was nothing to delete.
pub fn remove_old(exe: &Path) -> std::io::Result<bool> {
    match std::fs::remove_file(old_path(exe)) {
        Ok(()) => Ok(true),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(e),
    }
}

// ---------- the network: the app's only two requests -----------------------

/// Upper bound on the download. The exe is about 21 MB; a body far past that
/// is not it, and is not worth holding in memory to find out.
const MAX_DOWNLOAD: u64 = 64 * 1024 * 1024;

fn agent() -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(std::time::Duration::from_secs(10))
        .timeout_read(std::time::Duration::from_secs(30))
        .user_agent(concat!("nudge/", env!("CARGO_PKG_VERSION")))
        .build()
}

/// A failed request as the row says it: no network, or GitHub said no.
fn describe(e: ureq::Error) -> String {
    match e {
        ureq::Error::Status(code, _) => format!("GitHub refused the request (HTTP {code})"),
        ureq::Error::Transport(t) => match t.kind() {
            ureq::ErrorKind::Dns | ureq::ErrorKind::ConnectionFailed | ureq::ErrorKind::Io => {
                OFFLINE.into()
            }
            kind => format!("Could not reach GitHub: {kind}"),
        },
    }
}

/// The check: the latest release of this repository, as GitHub reports it.
pub fn fetch_latest() -> Result<Release, String> {
    let body = agent()
        .get(LATEST_URL)
        .set("Accept", "application/vnd.github+json")
        .call()
        .map_err(describe)?
        .into_string()
        .map_err(|_| UNREADABLE.to_string())?;
    parse_release(&body)
}

/// `Update`'s download: the bytes at `url`, bounded by [`MAX_DOWNLOAD`].
pub fn download(url: &str) -> Result<Vec<u8>, String> {
    use std::io::Read;
    let response = agent().get(url).call().map_err(describe)?;
    let mut bytes = Vec::new();
    response
        .into_reader()
        .take(MAX_DOWNLOAD + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| OFFLINE.to_string())?;
    if bytes.len() as u64 > MAX_DOWNLOAD {
        return Err("The download is too large to be Nudge".into());
    }
    Ok(bytes)
}

/// `Update`, start to finish: fetch the Windows exe, check it, put it at
/// `exe`'s path, then hand over — the tray that opened this window quits, and
/// only then does the new exe start.
#[cfg(target_os = "windows")]
pub fn install_release(
    release: &Release,
    exe: &Path,
    tray_thread: Option<u32>,
) -> Result<(), String> {
    let wanted = windows_download(release)?;
    let bytes = download(&wanted.url)?;
    verify(&bytes, &wanted.sha256)?;
    install_over(exe, &bytes)?;
    crate::platform::hand_over(exe, tray_thread)
}

/// `Open release page`: the page in the default browser.
pub fn open_page(url: &str) {
    #[cfg(target_os = "macos")]
    let opener = "open";
    #[cfg(target_os = "windows")]
    let opener = "explorer";
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    let opener = "xdg-open";
    if let Err(e) = std::process::Command::new(opener).arg(url).spawn() {
        eprintln!("[nudge-settings] could not open {url}: {e}");
    }
}

// ---------- the row (§9 "Updates") -----------------------------------------

/// What the row's button does once a newer release is found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Windows: download, verify, replace, restart.
    Install,
    /// macOS: the release's page in the browser.
    OpenPage,
}

/// This platform's action.
pub const ACTION: Action = if cfg!(target_os = "windows") {
    Action::Install
} else {
    Action::OpenPage
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Idle,
    Checking,
    UpToDate,
    Available(Release),
    Installing,
    /// The new exe is in place and started; the window is closing.
    Restarting,
    Failed(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    /// The row's one button, whose meaning is the state's.
    Press,
    Checked(Result<Release, String>),
    Installed(Result<(), String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    Nothing,
    Check,
    Install(Release),
    OpenPage(String),
    /// Close the window: the new version is starting.
    Close,
}

/// One input through the row's table. A result that arrives in a state that
/// did not ask for it changes nothing.
pub fn step(row: &Row, input: Input, current: &str, action: Action) -> (Row, Effect) {
    let stay = (row.clone(), Effect::Nothing);
    match (row, input) {
        (Row::Idle | Row::UpToDate | Row::Failed(_), Input::Press) => {
            (Row::Checking, Effect::Check)
        }
        (Row::Available(release), Input::Press) => match action {
            Action::Install => (Row::Installing, Effect::Install(release.clone())),
            Action::OpenPage => (row.clone(), Effect::OpenPage(release.html_url.clone())),
        },
        (Row::Checking, Input::Checked(Ok(release))) => {
            if is_newer(&release.tag_name, current) {
                (Row::Available(release), Effect::Nothing)
            } else {
                (Row::UpToDate, Effect::Nothing)
            }
        }
        (Row::Checking, Input::Checked(Err(reason)))
        | (Row::Installing, Input::Installed(Err(reason))) => {
            (Row::Failed(reason), Effect::Nothing)
        }
        (Row::Installing, Input::Installed(Ok(()))) => (Row::Restarting, Effect::Close),
        _ => stay,
    }
}

impl Row {
    /// The status left of the button; empty before the first press.
    pub fn status(&self) -> String {
        match self {
            Row::Idle => String::new(),
            Row::Checking => "Checking…".into(),
            Row::UpToDate => "Up to date".into(),
            Row::Available(release) => format!("{} available", version_of(&release.tag_name)),
            Row::Installing => "Updating…".into(),
            Row::Restarting => "Restarting…".into(),
            Row::Failed(reason) => reason.clone(),
        }
    }

    /// The button's label, and whether it can be pressed.
    pub fn button(&self, action: Action) -> (&'static str, bool) {
        match (self, action) {
            (Row::Available(_), Action::Install) => ("Update", true),
            (Row::Available(_), Action::OpenPage) => ("Open release page", true),
            (Row::Installing | Row::Restarting, _) => ("Update", false),
            (Row::Checking, _) => ("Check for updates", false),
            _ => ("Check for updates", true),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const HEX: &str = "b48b42654c96ae880e1fec6786ab7ad67f63a9dd44d99b2b815b84c10239694e";
    /// SHA-256 of `abc` (FIPS 180-2, appendix B.1).
    const ABC: &str = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";

    fn bytes_of(hex: &str) -> [u8; 32] {
        let mut out = [0; 32];
        for (i, b) in out.iter_mut().enumerate() {
            *b = u8::from_str_radix(&hex[2 * i..2 * i + 2], 16).unwrap();
        }
        out
    }

    fn release(tag: &str, assets: Vec<Asset>) -> Release {
        Release {
            tag_name: tag.into(),
            html_url: format!("https://github.com/Wizard1209/nudge/releases/tag/{tag}"),
            assets,
        }
    }

    fn exe(tag: &str, digest: Option<&str>) -> Asset {
        Asset {
            name: format!("nudge-{tag}-x86_64-windows.exe"),
            digest: digest.map(str::to_string),
            browser_download_url: format!("https://example.invalid/{tag}.exe"),
        }
    }

    // ----- version order: §9 "a newer release only" ------------------------

    #[test]
    fn a_higher_tag_is_newer_with_or_without_its_v() {
        assert!(is_newer("v0.3.0", "0.2.2"));
        assert!(is_newer("0.2.3", "0.2.2"));
        assert!(is_newer("v1.0.0", "0.99.99"));
        assert!(is_newer("v0.10.0", "0.9.0"), "numeric, not lexical");
    }

    #[test]
    fn the_same_or_an_older_latest_is_up_to_date() {
        assert!(!is_newer("v0.2.2", "0.2.2"));
        assert!(!is_newer("v0.2.1", "0.2.2"));
        assert!(!is_newer("v0.1.9", "0.2.2"));
    }

    #[test]
    fn a_release_outranks_a_pre_release_of_its_own_version() {
        assert!(is_newer("v0.3.0", "0.3.0-dev"));
        assert!(!is_newer("v0.3.0-rc.1", "0.3.0"));
        assert!(is_newer("v0.3.0-rc.2", "0.3.0-rc.1"));
        assert!(
            is_newer("v0.3.0-rc.10", "0.3.0-rc.9"),
            "numeric identifiers"
        );
        assert!(
            is_newer("v0.3.0-rc", "0.3.0-1"),
            "alphanumeric outranks numeric"
        );
        assert!(
            !is_newer("v0.3.0+build.7", "0.3.0"),
            "build metadata is ignored"
        );
    }

    #[test]
    fn a_tag_that_is_not_a_version_is_never_an_update() {
        for tag in ["nightly", "v0.3", "v0.3.0.1", "", "v", "v01.2.3x"] {
            assert!(!is_newer(tag, "0.2.2"), "{tag:?}");
        }
    }

    #[test]
    fn the_running_version_is_the_cargo_one() {
        assert_eq!(CURRENT, env!("CARGO_PKG_VERSION"));
        assert!(!is_newer(CURRENT, CURRENT));
    }

    #[test]
    fn the_row_shows_a_tag_without_its_v() {
        assert_eq!(version_of("v0.3.0"), "0.3.0");
        assert_eq!(version_of("0.3.0"), "0.3.0");
    }

    // ----- GitHub's reply --------------------------------------------------

    #[test]
    fn a_release_reads_the_fields_it_needs_and_ignores_the_rest() {
        let body = r#"{"tag_name":"v0.2.2","html_url":"https://github.com/Wizard1209/nudge/releases/tag/v0.2.2",
            "draft":false,"assets":[{"name":"nudge-v0.2.2-x86_64-windows.exe","size":21674489,
            "digest":"sha256:b48b42654c96ae880e1fec6786ab7ad67f63a9dd44d99b2b815b84c10239694e",
            "browser_download_url":"https://github.com/Wizard1209/nudge/releases/download/v0.2.2/nudge-v0.2.2-x86_64-windows.exe"}]}"#;
        let r = parse_release(body).unwrap();
        assert_eq!(r.tag_name, "v0.2.2");
        assert_eq!(r.assets.len(), 1);
        assert_eq!(
            r.assets[0].digest.as_deref(),
            Some(&*format!("sha256:{HEX}"))
        );
    }

    #[test]
    fn a_reply_that_is_not_a_release_is_unreadable() {
        for body in ["", "<html>", r#"{"message":"Not Found"}"#] {
            assert_eq!(
                parse_release(body).unwrap_err(),
                "GitHub sent a reply Nudge cannot read"
            );
        }
    }

    // ----- the Windows download --------------------------------------------

    #[test]
    fn the_windows_download_is_the_asset_named_for_the_tag() {
        let dmg = Asset {
            name: "nudge-v0.3.0-macos-universal.dmg".into(),
            ..exe("v0.3.0", Some(&format!("sha256:{ABC}")))
        };
        let r = release(
            "v0.3.0",
            vec![dmg, exe("v0.3.0", Some(&format!("sha256:{HEX}")))],
        );
        assert_eq!(
            windows_download(&r),
            Ok(Download {
                url: "https://example.invalid/v0.3.0.exe".into(),
                sha256: bytes_of(HEX),
            })
        );
    }

    #[test]
    fn a_release_without_a_windows_exe_is_refused() {
        let r = release(
            "v0.3.0",
            vec![exe("v0.2.9", Some(&format!("sha256:{HEX}")))],
        );
        assert_eq!(
            windows_download(&r).unwrap_err(),
            "The release has no Windows download"
        );
    }

    #[test]
    fn a_missing_or_unreadable_digest_refuses_the_update() {
        let short = format!("sha256:{}", &HEX[1..]);
        let not_hex = format!("sha256:{}", HEX.replace('b', "g"));
        let signed = format!("sha256:+{}", &HEX[1..]);
        for digest in [
            None,
            Some(""),
            Some("sha512:abc"),
            Some(&*short),
            Some(&*not_hex),
            Some(&*signed),
        ] {
            let r = release("v0.3.0", vec![exe("v0.3.0", digest)]);
            assert_eq!(
                windows_download(&r).unwrap_err(),
                "No checksum for the Windows download",
                "{digest:?}"
            );
        }
    }

    #[test]
    fn a_digest_reads_in_either_case() {
        let r = release(
            "v0.3.0",
            vec![exe(
                "v0.3.0",
                Some(&format!("sha256:{}", HEX.to_uppercase())),
            )],
        );
        assert_eq!(windows_download(&r).unwrap().sha256, bytes_of(HEX));
    }

    #[test]
    fn verify_accepts_the_matching_sha256_and_refuses_any_other() {
        assert_eq!(verify(b"abc", &bytes_of(ABC)), Ok(()));
        assert_eq!(
            verify(b"abd", &bytes_of(ABC)).unwrap_err(),
            "Download does not match its checksum"
        );
    }

    // ----- the replace sequence, on a directory ----------------------------

    #[test]
    fn install_puts_the_new_exe_at_the_path_and_keeps_the_old_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nudge.exe");
        std::fs::write(&path, b"old").unwrap();

        install_over(&path, b"new").unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"new");
        assert_eq!(std::fs::read(old_path(&path)).unwrap(), b"old");
        let names: Vec<_> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        assert_eq!(names.len(), 2, "nothing else is left behind: {names:?}");
    }

    #[test]
    fn the_old_exe_is_the_name_plus_old() {
        assert_eq!(
            old_path(Path::new("C:/apps/nudge-v0.2.2-x86_64-windows.exe")),
            Path::new("C:/apps/nudge-v0.2.2-x86_64-windows.exe.old")
        );
    }

    #[test]
    fn install_replaces_an_old_exe_left_from_an_earlier_update() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nudge.exe");
        std::fs::write(&path, b"v2").unwrap();
        std::fs::write(old_path(&path), b"v1").unwrap();

        install_over(&path, b"v3").unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"v3");
        assert_eq!(std::fs::read(old_path(&path)).unwrap(), b"v2");
    }

    #[test]
    fn a_failed_install_leaves_nothing_of_itself_behind() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nudge.exe"); // never created
        let err = install_over(&path, b"new").unwrap_err();
        assert!(err.starts_with("Could not replace Nudge: "), "{err}");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn the_new_app_removes_the_old_exe_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nudge.exe");
        std::fs::write(&path, b"new").unwrap();
        std::fs::write(old_path(&path), b"old").unwrap();

        assert!(remove_old(&path).unwrap());
        assert!(!old_path(&path).exists());
        assert!(path.exists());
        assert!(!remove_old(&path).unwrap(), "nothing left to remove");
    }

    // ----- the row's table -------------------------------------------------

    fn run(inputs: Vec<Input>, action: Action) -> (Row, Vec<Effect>) {
        let mut row = Row::Idle;
        let mut effects = Vec::new();
        for input in inputs {
            let (next, effect) = step(&row, input, "0.2.2", action);
            row = next;
            effects.push(effect);
        }
        (row, effects)
    }

    fn found(tag: &str) -> Input {
        Input::Checked(Ok(release(tag, vec![])))
    }

    #[test]
    fn nothing_is_asked_before_the_button_is_pressed() {
        assert_eq!(Row::Idle.status(), "");
        assert_eq!(Row::Idle.button(ACTION), ("Check for updates", true));
        assert_eq!(
            FOOTNOTE,
            "Checks github.com/Wizard1209/nudge for new releases, only when pressed."
        );
    }

    #[test]
    fn press_checks_and_greys_the_button() {
        let (row, effects) = run(vec![Input::Press], Action::Install);
        assert_eq!(row, Row::Checking);
        assert_eq!(effects, [Effect::Check]);
        assert_eq!(row.status(), "Checking…");
        assert_eq!(row.button(Action::Install), ("Check for updates", false));
        // A second press while checking asks nothing more.
        let (_, again) = step(&row, Input::Press, "0.2.2", Action::Install);
        assert_eq!(again, Effect::Nothing);
    }

    #[test]
    fn a_newer_release_offers_this_platforms_action() {
        let (row, _) = run(vec![Input::Press, found("v0.3.0")], Action::Install);
        assert_eq!(row.status(), "0.3.0 available");
        assert_eq!(row.button(Action::Install), ("Update", true));
        assert_eq!(row.button(Action::OpenPage), ("Open release page", true));
    }

    #[test]
    fn the_same_or_an_older_release_is_up_to_date_and_can_be_checked_again() {
        for tag in ["v0.2.2", "v0.2.1"] {
            let (row, _) = run(vec![Input::Press, found(tag)], Action::Install);
            assert_eq!(row, Row::UpToDate);
            assert_eq!(row.status(), "Up to date");
            assert_eq!(row.button(Action::Install), ("Check for updates", true));
            let (next, effect) = step(&row, Input::Press, "0.2.2", Action::Install);
            assert_eq!((next, effect), (Row::Checking, Effect::Check));
        }
    }

    #[test]
    fn a_failed_check_shows_its_reason_and_can_be_retried() {
        let offline = "Offline: could not reach GitHub".to_string();
        let (row, effects) = run(
            vec![Input::Press, Input::Checked(Err(offline.clone()))],
            Action::Install,
        );
        assert_eq!(row.status(), offline);
        assert_eq!(row.button(Action::Install), ("Check for updates", true));
        assert_eq!(effects, [Effect::Check, Effect::Nothing]);
        assert_eq!(
            step(&row, Input::Press, "0.2.2", Action::Install).1,
            Effect::Check
        );
    }

    #[test]
    fn update_installs_then_closes_the_window_for_the_restart() {
        let (row, effects) = run(
            vec![
                Input::Press,
                found("v0.3.0"),
                Input::Press,
                Input::Installed(Ok(())),
            ],
            Action::Install,
        );
        assert_eq!(
            effects,
            [
                Effect::Check,
                Effect::Nothing,
                Effect::Install(release("v0.3.0", vec![])),
                Effect::Close
            ]
        );
        assert_eq!(row, Row::Restarting);
    }

    #[test]
    fn the_button_is_greyed_while_updating() {
        let (row, _) = run(
            vec![Input::Press, found("v0.3.0"), Input::Press],
            Action::Install,
        );
        assert_eq!(row, Row::Installing);
        assert_eq!(row.status(), "Updating…");
        assert_eq!(row.button(Action::Install), ("Update", false));
        assert_eq!(
            step(&row, Input::Press, "0.2.2", Action::Install).1,
            Effect::Nothing
        );
    }

    #[test]
    fn a_refused_install_shows_its_reason_and_offers_a_fresh_check() {
        let reason = "Download does not match its checksum".to_string();
        let (row, _) = run(
            vec![
                Input::Press,
                found("v0.3.0"),
                Input::Press,
                Input::Installed(Err(reason.clone())),
            ],
            Action::Install,
        );
        assert_eq!(row.status(), reason);
        assert_eq!(row.button(Action::Install), ("Check for updates", true));
    }

    #[test]
    fn open_release_page_opens_it_and_stays() {
        let (row, effects) = run(
            vec![Input::Press, found("v0.3.0"), Input::Press],
            Action::OpenPage,
        );
        assert_eq!(
            effects[2],
            Effect::OpenPage("https://github.com/Wizard1209/nudge/releases/tag/v0.3.0".into())
        );
        assert_eq!(row, Row::Available(release("v0.3.0", vec![])));
    }

    #[test]
    fn a_result_nobody_asked_for_changes_nothing() {
        assert_eq!(run(vec![found("v0.3.0")], Action::Install).0, Row::Idle);
        assert_eq!(
            run(vec![Input::Installed(Ok(()))], Action::Install),
            (Row::Idle, vec![Effect::Nothing])
        );
        let (row, _) = run(vec![Input::Press, found("v0.2.2")], Action::Install);
        assert_eq!(step(&row, found("v0.3.0"), "0.2.2", Action::Install).0, row);
    }

    // ----- what a failed request says --------------------------------------

    #[test]
    fn a_refusal_names_githubs_status() {
        let refused = ureq::Error::Status(403, ureq::Response::new(403, "Forbidden", "").unwrap());
        assert_eq!(describe(refused), "GitHub refused the request (HTTP 403)");
    }

    #[test]
    fn no_connection_reads_as_offline() {
        // Port 1 on loopback: refused at once, and nothing leaves the machine.
        let err = agent().get("http://127.0.0.1:1/").call().unwrap_err();
        assert_eq!(describe(err), "Offline: could not reach GitHub");
    }
}
