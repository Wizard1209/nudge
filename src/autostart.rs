//! Launch-at-login autostart.
//!
//! Two layers, deliberately split:
//!
//! - [`AutostartProvider`] is the OS mechanism that actually makes the app
//!   start at login. Windows uses the `HKCU\…\Run` registry value; other
//!   platforms can grow their own impls (launch agents, `.desktop`, …). It's a
//!   trait so tests and the WASM build substitute an in-memory fake.
//!
//! - [`apply_autostart`] is the *transactional* rule on top: perform the OS
//!   change, confirm it through the system, and only then write `autostart`
//!   into the config. config.json therefore only ever claims a state the OS
//!   actually has — a registry write that silently fails never leaves a
//!   `true` lie in the file. This is what lets the rest of the app treat the
//!   config as the source of truth.

use crate::config::{Config, ConfigError};

/// The OS mechanism for launch-at-login. Errors carry a human-readable detail
/// string (the underlying registry / OS error) for surfacing to the user.
pub trait AutostartProvider {
    /// Register the app to launch at login.
    fn enable(&self) -> Result<(), String>;
    /// Unregister. Disabling something already disabled must succeed (the
    /// post-condition "not registered" already holds).
    fn disable(&self) -> Result<(), String>;
    /// Whether the app is currently registered. Used both to drive the UI
    /// toggle and to *confirm* an `enable`/`disable` actually took effect.
    fn is_enabled(&self) -> Result<bool, String>;
}

/// Why an [`apply_autostart`] call failed. Split so the caller can tell
/// "the OS refused" (nothing changed, config untouched) from "the OS change
/// stuck but persisting the config failed" (system and config now disagree on
/// disk until the next save).
#[derive(Debug)]
pub enum AutostartError {
    /// The provider call failed, or the system didn't confirm the change.
    /// Config was left untouched.
    Backend(String),
    /// The OS change succeeded and was confirmed, but writing the config
    /// back failed.
    Persist(ConfigError),
}

impl std::fmt::Display for AutostartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AutostartError::Backend(d) => write!(f, "autostart: {d}"),
            AutostartError::Persist(e) => write!(f, "autostart persisted change failed: {e}"),
        }
    }
}

impl std::error::Error for AutostartError {}

/// Transactionally move autostart to `desired`.
///
/// Order is load-bearing: OS change → confirm via `is_enabled` → mutate
/// `config` → persist. If the OS change fails or isn't confirmed, `config` is
/// left exactly as it was and `persist` is never called — so a failed registry
/// write can't produce a config that claims autostart is on. Persistence is
/// injected as a closure so this stays platform-agnostic and unit-testable
/// (native passes `config::save`, WASM passes `config::save_to_localstorage`).
pub fn apply_autostart<F>(
    provider: &dyn AutostartProvider,
    config: &mut Config,
    desired: bool,
    persist: F,
) -> Result<(), AutostartError>
where
    F: FnOnce(&Config) -> Result<(), ConfigError>,
{
    let os_result = if desired {
        provider.enable()
    } else {
        provider.disable()
    };
    os_result.map_err(AutostartError::Backend)?;

    let confirmed = provider.is_enabled().map_err(AutostartError::Backend)?;
    if confirmed != desired {
        return Err(AutostartError::Backend(format!(
            "system did not confirm autostart={desired} (is_enabled reports {confirmed})"
        )));
    }

    config.autostart = desired;
    persist(config).map_err(AutostartError::Persist)
}

/// In-memory provider. Used by tests, by the WASM build (no registry and no
/// launchd in the browser), and as the fallback on platforms with no real
/// backend yet, so the linux dev build compiles.
///
/// It is never used in a shipped desktop binary. Windows always goes through
/// `WindowsRegistryProvider` and macOS through `MacLaunchAgentProvider`; if
/// either can't be constructed we refuse to open the settings window rather
/// than silently lie about the OS state. The cfg excludes both of those
/// targets for exactly that reason — a fake that cannot be reached is better
/// than one that can.
#[cfg(any(test, not(any(target_os = "windows", target_os = "macos"))))]
pub struct FakeProvider {
    enabled: std::cell::Cell<bool>,
    fail_enable: bool,
    fail_disable: bool,
    /// When true, `is_enabled` reports the *opposite* of the tracked state —
    /// simulates an OS change that silently didn't stick.
    lie_on_confirm: bool,
}

#[cfg(any(test, not(any(target_os = "windows", target_os = "macos"))))]
impl FakeProvider {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled: std::cell::Cell::new(enabled),
            fail_enable: false,
            fail_disable: false,
            lie_on_confirm: false,
        }
    }
    // The failure-injecting constructors are exercised only by unit tests
    // (settings_app + autostart). They stay cfg(test) so the release binary
    // doesn't carry dead public API.
    #[cfg(test)]
    pub fn failing_enable() -> Self {
        Self {
            fail_enable: true,
            ..Self::new(false)
        }
    }
    #[cfg(test)]
    pub fn unconfirmed() -> Self {
        Self {
            lie_on_confirm: true,
            ..Self::new(false)
        }
    }
}

#[cfg(any(test, not(any(target_os = "windows", target_os = "macos"))))]
impl AutostartProvider for FakeProvider {
    fn enable(&self) -> Result<(), String> {
        if self.fail_enable {
            return Err("fake enable failed".to_string());
        }
        self.enabled.set(true);
        Ok(())
    }
    fn disable(&self) -> Result<(), String> {
        if self.fail_disable {
            return Err("fake disable failed".to_string());
        }
        self.enabled.set(false);
        Ok(())
    }
    fn is_enabled(&self) -> Result<bool, String> {
        Ok(self.enabled.get() ^ self.lie_on_confirm)
    }
}

/// Registry-name the app registers itself under in the `Run` key. Stable
/// identity so enable/disable/is_enabled all address the same value.
#[cfg(target_os = "windows")]
pub const RUN_VALUE_NAME: &str = "Nudge";

/// The per-user autostart key Windows reads at login.
#[cfg(target_os = "windows")]
pub const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

/// Windows autostart via the per-user `HKCU\…\Run` registry value. Storing the
/// value makes Windows launch `<exe>` at login; deleting it stops that.
/// `is_enabled` checks only for the value's presence (not that it still points
/// at the current exe), which is enough to drive the toggle.
#[cfg(target_os = "windows")]
pub struct WindowsRegistryProvider {
    /// Command line written into the Run value. Defaults to the current exe.
    exe: std::path::PathBuf,
}

#[cfg(target_os = "windows")]
impl WindowsRegistryProvider {
    /// Provider targeting the currently running executable.
    pub fn for_current_exe() -> Result<Self, String> {
        let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
        Ok(Self { exe })
    }
}

#[cfg(target_os = "windows")]
fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(target_os = "windows")]
impl AutostartProvider for WindowsRegistryProvider {
    fn enable(&self) -> Result<(), String> {
        use windows::Win32::Foundation::ERROR_SUCCESS;
        use windows::Win32::System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegOpenKeyExW,
            RegSetValueExW,
        };
        use windows::core::PCWSTR;

        let subkey = to_wide(RUN_SUBKEY);
        let name = to_wide(RUN_VALUE_NAME);
        // The Run value is a command line; quote the path so spaces survive.
        let data = to_wide(&format!("\"{}\"", self.exe.display()));
        let data_bytes = unsafe {
            std::slice::from_raw_parts(data.as_ptr() as *const u8, std::mem::size_of_val(&data[..]))
        };

        unsafe {
            let mut hkey = HKEY::default();
            let rc = RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                Some(0),
                KEY_SET_VALUE,
                &mut hkey,
            );
            if rc != ERROR_SUCCESS {
                return Err(format!("RegOpenKeyExW(Run): {rc:?}"));
            }
            let set = RegSetValueExW(
                hkey,
                PCWSTR(name.as_ptr()),
                Some(0),
                REG_SZ,
                Some(data_bytes),
            );
            let _ = RegCloseKey(hkey);
            if set != ERROR_SUCCESS {
                return Err(format!("RegSetValueExW(Nudge): {set:?}"));
            }
        }
        Ok(())
    }

    fn disable(&self) -> Result<(), String> {
        use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
        use windows::Win32::System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, RegCloseKey, RegDeleteValueW, RegOpenKeyExW,
        };
        use windows::core::PCWSTR;

        let subkey = to_wide(RUN_SUBKEY);
        let name = to_wide(RUN_VALUE_NAME);

        unsafe {
            let mut hkey = HKEY::default();
            let rc = RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                Some(0),
                KEY_SET_VALUE,
                &mut hkey,
            );
            if rc != ERROR_SUCCESS {
                return Err(format!("RegOpenKeyExW(Run): {rc:?}"));
            }
            let del = RegDeleteValueW(hkey, PCWSTR(name.as_ptr()));
            let _ = RegCloseKey(hkey);
            // Deleting an already-absent value is a no-op success: the
            // post-condition "not registered" already holds.
            if del != ERROR_SUCCESS && del != ERROR_FILE_NOT_FOUND {
                return Err(format!("RegDeleteValueW(Nudge): {del:?}"));
            }
        }
        Ok(())
    }

    fn is_enabled(&self) -> Result<bool, String> {
        use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
        use windows::Win32::System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, RegCloseKey, RegOpenKeyExW, RegQueryValueExW,
        };
        use windows::core::PCWSTR;

        let subkey = to_wide(RUN_SUBKEY);
        let name = to_wide(RUN_VALUE_NAME);

        unsafe {
            let mut hkey = HKEY::default();
            let rc = RegOpenKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                Some(0),
                KEY_QUERY_VALUE,
                &mut hkey,
            );
            if rc != ERROR_SUCCESS {
                return Err(format!("RegOpenKeyExW(Run): {rc:?}"));
            }
            // Querying with null data buffers just probes existence.
            let q = RegQueryValueExW(hkey, PCWSTR(name.as_ptr()), None, None, None, None);
            let _ = RegCloseKey(hkey);
            if q == ERROR_SUCCESS {
                Ok(true)
            } else if q == ERROR_FILE_NOT_FOUND {
                Ok(false)
            } else {
                Err(format!("RegQueryValueExW(Nudge): {q:?}"))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// macOS: LaunchAgent plist
// ---------------------------------------------------------------------------

/// launchd job label. Same reverse-DNS identity as `CFBundleIdentifier` in
/// `scripts/build-macos-app.sh`, and it doubles as the plist filename — so
/// enable/disable/is_enabled all address one file and one job.
#[cfg(target_os = "macos")]
pub const LAUNCH_AGENT_LABEL: &str = "com.wizard1209.nudge";

/// macOS autostart via a per-user LaunchAgent: a plist in
/// `~/Library/LaunchAgents/` that launchd loads at login.
///
/// # Why not `SMAppService`
///
/// `SMAppService.mainApp` is the modern API and it is the wrong one here. It
/// registers a *bundle identity* — the OS remembers the app by its code
/// signature, and a registration made by one build is not honoured for the
/// next one unless the signing identity is stable. This project ships an
/// ad-hoc signature (`codesign -s -`, see `scripts/build-macos-app.sh`); there
/// is no Developer ID, so the code identity changes from build to build and
/// registrations go stale in ways that look exactly like "autostart silently
/// stopped working". A LaunchAgent is *path*-based: it names an executable on
/// disk and launchd does not care who signed it. That is what makes it work
/// under ad-hoc signing. Do not "modernise" this to SMAppService before a
/// Developer ID cert exists.
#[cfg(target_os = "macos")]
pub struct MacLaunchAgentProvider {
    /// Absolute path launchd will exec. See [`Self::for_current_exe`].
    program: std::path::PathBuf,
    /// `~/Library/LaunchAgents/<label>.plist`.
    plist_path: std::path::PathBuf,
}

#[cfg(target_os = "macos")]
impl MacLaunchAgentProvider {
    /// Provider targeting the currently running executable.
    ///
    /// Inside a bundle this resolves to `Nudge.app/Contents/MacOS/nudge`,
    /// which is exactly the path launchd must exec — launching the inner
    /// binary directly still gives the app its bundle (`NSBundle.mainBundle`
    /// walks up from the executable), so there is no need to shell out to
    /// `open -a`. A *bare* binary works just as well, since the plist only
    /// ever names a path; that is what makes this provider exercisable in
    /// tests and from `cargo run` without building a bundle first.
    ///
    /// The path is canonicalised so a symlinked launch (`/usr/local/bin/nudge`
    /// pointing into a bundle, a `target/debug` symlink) records the real file
    /// rather than a link that may be repointed later.
    pub fn for_current_exe() -> Result<Self, String> {
        let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
        // canonicalize can fail on an exotic filesystem; the un-resolved path
        // is still a usable ProgramArguments entry, so degrade rather than
        // refuse to construct.
        let program = std::fs::canonicalize(&exe).unwrap_or(exe);

        let home = dirs::home_dir().ok_or_else(|| "cannot resolve home directory".to_string())?;
        Ok(Self {
            plist_path: launch_agent_plist_path(&home, LAUNCH_AGENT_LABEL),
            program,
        })
    }

    /// Where this provider writes its plist.
    pub fn plist_path(&self) -> &std::path::Path {
        &self.plist_path
    }

    /// The executable path this provider records in the plist.
    pub fn program(&self) -> &std::path::Path {
        &self.program
    }

    /// Write `~/Library/LaunchAgents/<label>.plist`, creating the directory if
    /// needed. Split out from [`AutostartProvider::enable`] because it is the
    /// only half that can be sandboxed: the plist lands under `$HOME`, while
    /// `launchctl` always talks to the live per-user launchd domain (see
    /// `tests/macos_autostart.rs`).
    pub fn write_plist(&self) -> Result<(), String> {
        if let Some(dir) = self.plist_path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        }
        std::fs::write(
            &self.plist_path,
            launch_agent_plist_xml(LAUNCH_AGENT_LABEL, &self.program),
        )
        .map_err(|e| format!("write {}: {e}", self.plist_path.display()))
    }

    /// Delete the plist. Returns `Ok(false)` when there was nothing to delete —
    /// that is success, not an error: the post-condition "no launch agent" holds.
    pub fn remove_plist(&self) -> Result<bool, String> {
        match std::fs::remove_file(&self.plist_path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(format!("remove {}: {e}", self.plist_path.display())),
        }
    }

    /// `launchctl bootstrap gui/<uid> <plist>`, falling back to the pre-10.11
    /// `launchctl load -w <plist>` when the running launchctl has no
    /// `bootstrap` subcommand.
    ///
    /// # Why it boots the label out first
    ///
    /// Bootstrapping a label the domain already holds is an error, and — this
    /// was measured, not assumed — launchctl on macOS 26 reports it as a bare
    /// `Bootstrap failed: 5: Input/output error`, with no distinguishing text.
    /// Exit 5 is also what a genuinely broken plist yields, so the reply cannot
    /// be classified after the fact: tolerating 5 would swallow real failures,
    /// and rejecting it would make a second `enable()` fail. Removing the
    /// precondition is the only way out — after a best-effort `bootout`, the
    /// domain is known not to hold the label, so any bootstrap error left is a
    /// real one.
    ///
    /// It is also the correct behaviour independent of that: bootstrap ignores
    /// a plist whose label is already loaded, so re-enabling after the app
    /// moved (`~/Downloads/Nudge.app` → `/Applications`) would otherwise leave
    /// launchd pointed at the old path.
    fn register(&self) -> Result<(), String> {
        let _ = self.unregister();

        let domain = format!("gui/{}", current_uid());
        let out = run_launchctl(&["bootstrap", &domain, &self.plist_path.to_string_lossy()])?;
        if out.ok || launchctl_says_already_registered(out.code, &out.text) {
            return Ok(());
        }
        if launchctl_says_unknown_subcommand(&out.text) {
            let legacy = run_launchctl(&["load", "-w", &self.plist_path.to_string_lossy()])?;
            if legacy.ok || launchctl_says_already_registered(legacy.code, &legacy.text) {
                return Ok(());
            }
            return Err(format!("launchctl load: {}", legacy.text));
        }
        Err(format!("launchctl bootstrap: {}", out.text))
    }

    /// `launchctl bootout gui/<uid>/<label>`, with the pre-10.11 `unload`
    /// fallback. "No such process" is success: the post-condition "not in the
    /// domain" already holds.
    fn unregister(&self) -> Result<(), String> {
        let target = format!("gui/{}/{}", current_uid(), LAUNCH_AGENT_LABEL);
        let out = run_launchctl(&["bootout", &target])?;
        if out.ok || launchctl_says_not_registered(out.code, &out.text) {
            return Ok(());
        }
        if launchctl_says_unknown_subcommand(&out.text) {
            let legacy = run_launchctl(&["unload", "-w", &self.plist_path.to_string_lossy()])?;
            if legacy.ok || launchctl_says_not_registered(legacy.code, &legacy.text) {
                return Ok(());
            }
            return Err(format!("launchctl unload: {}", legacy.text));
        }
        Err(format!("launchctl bootout: {}", out.text))
    }
}

/// `~/Library/LaunchAgents/<label>.plist` — the directory launchd scans for
/// per-user agents at login. Pure so the path derivation is testable without
/// touching a real home directory.
#[cfg(target_os = "macos")]
pub fn launch_agent_plist_path(home: &std::path::Path, label: &str) -> std::path::PathBuf {
    home.join("Library/LaunchAgents")
        .join(format!("{label}.plist"))
}

/// The three keys that make a launch-at-login agent: `Label` (job identity),
/// `ProgramArguments` (what to exec), `RunAtLoad` (do it when launchd loads
/// the job, which at login is immediately).
#[cfg(target_os = "macos")]
pub fn launch_agent_plist_xml(label: &str, program: &std::path::Path) -> String {
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{label}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{program}</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
</dict>
</plist>
"#,
        label = xml_escape(label),
        program = xml_escape(&program.to_string_lossy()),
    )
}

/// Escape the five XML metacharacters. Paths legitimately contain `&` and
/// `<` (a directory may be named anything), and an unescaped one produces a
/// plist launchd refuses to parse — which surfaces as a bootstrap failure far
/// from its cause.
#[cfg(target_os = "macos")]
fn xml_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
        }
    }
    out
}

/// Extract the `Label` value from plist XML. Deliberately a scan for the
/// `<key>Label</key>` / `<string>…</string>` pair rather than a real plist
/// parser: the only file this ever reads is one we wrote ourselves, and the
/// question being asked is narrow — "is this our agent, or did something else
/// take the filename?".
#[cfg(target_os = "macos")]
pub fn parse_plist_label(xml: &str) -> Option<String> {
    let after_key = xml.split_once("<key>Label</key>")?.1;
    let open = after_key.find("<string>")? + "<string>".len();
    let close = after_key[open..].find("</string>")? + open;
    Some(after_key[open..close].trim().to_string())
}

/// Captured result of one `launchctl` invocation. `text` merges stdout and
/// stderr because launchctl is inconsistent about which one carries the
/// diagnostic.
#[cfg(target_os = "macos")]
struct LaunchctlOutput {
    ok: bool,
    code: Option<i32>,
    text: String,
}

#[cfg(target_os = "macos")]
fn run_launchctl(args: &[&str]) -> Result<LaunchctlOutput, String> {
    let out = std::process::Command::new("launchctl")
        .args(args)
        .output()
        .map_err(|e| format!("launchctl {}: {e}", args.join(" ")))?;
    let mut text = String::from_utf8_lossy(&out.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stdout = stdout.trim();
    if !stdout.is_empty() {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(stdout);
    }
    if text.is_empty() {
        text = format!("exit {:?}", out.status.code());
    }
    Ok(LaunchctlOutput {
        ok: out.status.success(),
        code: out.status.code(),
        text,
    })
}

/// The job is already in the domain, on the launchctl versions that say so
/// distinctly: `EEXIST` (17), `EINPROGRESS` (37) while a previous bootstrap of
/// the same label settles, or an explicit "already loaded" from the legacy
/// `load` path.
///
/// Belt and braces rather than the mechanism — `register` boots the label out
/// first precisely because macOS 26's launchctl does *not* say so distinctly
/// (measured: a bare `Bootstrap failed: 5: Input/output error`). Exit 5 is
/// deliberately absent from this list: it is equally what a malformed plist
/// returns, so accepting it would turn a broken agent into a reported success.
#[cfg(target_os = "macos")]
fn launchctl_says_already_registered(code: Option<i32>, text: &str) -> bool {
    matches!(code, Some(17) | Some(37))
        || text.contains("already bootstrapped")
        || text.contains("already loaded")
        || text.contains("Already loaded")
}

/// Nothing to unregister — `ESRCH` (3) / "Could not find specified service".
/// Disabling something already disabled must succeed.
#[cfg(target_os = "macos")]
fn launchctl_says_not_registered(code: Option<i32>, text: &str) -> bool {
    matches!(code, Some(3) | Some(113))
        || text.contains("Could not find specified service")
        || text.contains("No such process")
        || text.contains("not find service")
}

/// This launchctl predates `bootstrap`/`bootout` (macOS < 10.11).
#[cfg(target_os = "macos")]
fn launchctl_says_unknown_subcommand(text: &str) -> bool {
    text.contains("Unrecognized subcommand") || text.contains("Unknown subcommand")
}

/// Real uid of this process. launchd domain targets are `gui/<uid>`; `$UID` is
/// a shell variable and is not in a spawned process's environment, so ask the
/// kernel. `getuid` cannot fail and needs no libc dependency for one symbol.
#[cfg(target_os = "macos")]
fn current_uid() -> u32 {
    unsafe extern "C" {
        fn getuid() -> u32;
    }
    unsafe { getuid() }
}

#[cfg(target_os = "macos")]
impl AutostartProvider for MacLaunchAgentProvider {
    fn enable(&self) -> Result<(), String> {
        self.write_plist()?;
        if let Err(e) = self.register() {
            // Roll the file back. launchd validates the plist at bootstrap
            // time, so a failure here usually means the file we just wrote is
            // unusable — leaving it behind would make `is_enabled` report
            // `true` for an agent that cannot actually run, and
            // `apply_autostart` would then persist that lie.
            let _ = self.remove_plist();
            return Err(e);
        }
        Ok(())
    }

    fn disable(&self) -> Result<(), String> {
        // A bootout failure is deliberately non-fatal, unlike the bootstrap
        // failure above. Unregistering only affects the *current* login
        // session; what decides whether the app comes back at the next login
        // is the file, and removing it below is the step that must succeed.
        // Hard-failing here would leave the plist on disk with the caller told
        // "disable failed" — the worse of the two states, and the one that
        // makes `is_enabled` and the config disagree.
        let _ = self.unregister();
        self.remove_plist()?;
        Ok(())
    }

    /// Reports whether *our* plist is present in `~/Library/LaunchAgents`.
    ///
    /// This is read from the file rather than from `launchctl print` on
    /// purpose, and the choice matters because `apply_autostart` uses this
    /// call to *confirm* a write. The two sources answer different questions:
    /// the plist is the durable record launchd scans at every login — it is
    /// precisely the thing that decides "will Nudge start next time I log
    /// in?" — while `launchctl print gui/<uid>/<label>` describes the current
    /// boot session's domain, which drifts from that answer in both
    /// directions. A job can be loaded now from a plist that has since been
    /// deleted (stays until logout), and a plist can be on disk without ever
    /// being bootstrapped (no GUI session, e.g. over ssh), which would make
    /// `is_enabled` report `false` for an app that will in fact autostart.
    /// Confirming against the file means `enable` is confirmed against the
    /// exact artifact `enable` wrote.
    ///
    /// The label is checked, not just the filename, so a foreign file that
    /// happens to occupy the path isn't reported as our registration.
    fn is_enabled(&self) -> Result<bool, String> {
        match std::fs::read_to_string(&self.plist_path) {
            Ok(xml) => Ok(parse_plist_label(&xml).as_deref() == Some(LAUNCH_AGENT_LABEL)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(format!("read {}: {e}", self.plist_path.display())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn recording_persist(
        sink: &std::cell::RefCell<Option<Config>>,
    ) -> impl FnOnce(&Config) -> Result<(), ConfigError> + '_ {
        move |c: &Config| {
            *sink.borrow_mut() = Some(c.clone());
            Ok(())
        }
    }

    #[test]
    fn enable_success_sets_and_persists_config() {
        let provider = FakeProvider::new(false);
        let mut config = Config::default();
        let saved = std::cell::RefCell::new(None);

        apply_autostart(&provider, &mut config, true, recording_persist(&saved)).unwrap();

        assert!(config.autostart, "in-memory config reflects the change");
        assert!(provider.is_enabled().unwrap(), "OS state actually enabled");
        assert_eq!(
            saved.borrow().as_ref().map(|c| c.autostart),
            Some(true),
            "persisted config carries autostart=true"
        );
    }

    #[test]
    fn disable_success_clears_and_persists_config() {
        let provider = FakeProvider::new(true);
        let mut config = Config {
            autostart: true,
            ..Config::default()
        };
        let saved = std::cell::RefCell::new(None);

        apply_autostart(&provider, &mut config, false, recording_persist(&saved)).unwrap();

        assert!(!config.autostart);
        assert!(!provider.is_enabled().unwrap());
        assert_eq!(saved.borrow().as_ref().map(|c| c.autostart), Some(false));
    }

    #[test]
    fn backend_failure_leaves_config_untouched_and_skips_persist() {
        // The registry write fails: config must NOT flip, and persist must
        // never run — otherwise config.json would claim a state the OS
        // doesn't have.
        let provider = FakeProvider::failing_enable();
        let mut config = Config::default();
        let saved = std::cell::RefCell::new(None);

        let err =
            apply_autostart(&provider, &mut config, true, recording_persist(&saved)).unwrap_err();

        assert!(matches!(err, AutostartError::Backend(_)));
        assert!(!config.autostart, "config not mutated on backend failure");
        assert!(
            saved.borrow().is_none(),
            "persist never called on backend failure"
        );
    }

    #[test]
    fn unconfirmed_change_is_a_backend_error() {
        // enable() returns Ok but is_enabled disagrees → we must not trust it.
        let provider = FakeProvider::unconfirmed();
        let mut config = Config::default();
        let saved = std::cell::RefCell::new(None);

        let err =
            apply_autostart(&provider, &mut config, true, recording_persist(&saved)).unwrap_err();

        assert!(matches!(err, AutostartError::Backend(_)));
        assert!(!config.autostart);
        assert!(saved.borrow().is_none());
    }

    #[test]
    fn persist_failure_surfaces_as_persist_error() {
        // OS change stuck and confirmed, but the disk write failed.
        let provider = FakeProvider::new(false);
        let mut config = Config::default();

        let err = apply_autostart(&provider, &mut config, true, |_c| {
            Err(ConfigError::Io {
                path: "x".to_string(),
                detail: "disk full".to_string(),
            })
        })
        .unwrap_err();

        assert!(matches!(err, AutostartError::Persist(_)));
        assert!(provider.is_enabled().unwrap(), "OS change still happened");
    }

    #[test]
    fn enable_is_idempotent() {
        let provider = FakeProvider::new(true); // already enabled
        let mut config = Config::default();
        let saved = std::cell::RefCell::new(None);

        apply_autostart(&provider, &mut config, true, recording_persist(&saved)).unwrap();

        assert!(config.autostart);
        assert_eq!(saved.borrow().as_ref().map(|c| c.autostart), Some(true));
    }

    // -----------------------------------------------------------------
    // macOS LaunchAgent: the pure halves (XML, paths, launchctl replies)
    // -----------------------------------------------------------------

    #[cfg(target_os = "macos")]
    #[test]
    fn plist_path_is_under_library_launchagents() {
        let path = launch_agent_plist_path(
            std::path::Path::new("/Users/someone"),
            "com.wizard1209.nudge",
        );
        assert_eq!(
            path,
            std::path::PathBuf::from(
                "/Users/someone/Library/LaunchAgents/com.wizard1209.nudge.plist"
            )
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn plist_xml_carries_label_program_and_runatload() {
        let xml = launch_agent_plist_xml(
            "com.wizard1209.nudge",
            std::path::Path::new("/Applications/Nudge.app/Contents/MacOS/nudge"),
        );

        assert!(xml.starts_with("<?xml version=\"1.0\" encoding=\"UTF-8\"?>"));
        assert!(xml.contains("<key>Label</key>\n\t<string>com.wizard1209.nudge</string>"));
        assert!(xml.contains("<string>/Applications/Nudge.app/Contents/MacOS/nudge</string>"));
        assert!(xml.contains("<key>RunAtLoad</key>\n\t<true/>"));
        assert!(xml.trim_end().ends_with("</plist>"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn plist_xml_escapes_metacharacters_in_the_path() {
        // A directory may be named anything; an unescaped `&` yields a plist
        // launchd refuses to parse, and the failure surfaces at bootstrap
        // time, far from its cause.
        let xml = launch_agent_plist_xml(
            "com.wizard1209.nudge",
            std::path::Path::new("/Users/a&b/<odd>/nudge"),
        );
        assert!(xml.contains("<string>/Users/a&amp;b/&lt;odd&gt;/nudge</string>"));
        assert!(!xml.contains("/Users/a&b/"));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn label_round_trips_through_the_generated_plist() {
        let xml = launch_agent_plist_xml("com.wizard1209.nudge", std::path::Path::new("/bin/true"));
        assert_eq!(
            parse_plist_label(&xml).as_deref(),
            Some("com.wizard1209.nudge")
        );
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn label_of_a_foreign_or_malformed_plist_is_not_ours() {
        let foreign =
            launch_agent_plist_xml("com.example.other", std::path::Path::new("/bin/true"));
        assert_eq!(
            parse_plist_label(&foreign).as_deref(),
            Some("com.example.other")
        );
        assert_eq!(parse_plist_label("not a plist at all"), None);
        assert_eq!(parse_plist_label("<key>Label</key>"), None);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn already_registered_is_recognised_as_success() {
        // EEXIST / EINPROGRESS, plus the textual forms across macOS versions.
        assert!(launchctl_says_already_registered(
            Some(17),
            "Bootstrap failed: 17: File exists"
        ));
        assert!(launchctl_says_already_registered(
            Some(37),
            "Bootstrap failed: 37: Operation already in progress"
        ));
        assert!(launchctl_says_already_registered(
            Some(5),
            "service already bootstrapped"
        ));
    }

    /// Observed on macOS 26: re-bootstrapping a loaded label yields exactly
    /// `Bootstrap failed: 5: Input/output error` — the same reply a malformed
    /// plist gets. Classifying it as "already registered" would report success
    /// for an agent that cannot run, so `register` boots the label out first
    /// instead and this reply stays a failure.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_ambiguous_exit_5_is_not_treated_as_already_registered() {
        assert!(!launchctl_says_already_registered(
            Some(5),
            "Bootstrap failed: 5: Input/output error\nTry re-running the command as root \
             for richer errors."
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn not_registered_is_recognised_as_success_for_disable() {
        // Observed on macOS 26 for a bootout of a label that is not loaded.
        assert!(launchctl_says_not_registered(
            Some(3),
            "Boot-out failed: 3: No such process"
        ));
        assert!(launchctl_says_not_registered(
            Some(113),
            "Could not find specified service"
        ));
        assert!(!launchctl_says_not_registered(
            Some(5),
            "Boot-out failed: 5: Input/output error"
        ));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn pre_bootstrap_launchctl_is_detected() {
        assert!(launchctl_says_unknown_subcommand(
            "launchctl: Unrecognized subcommand: bootstrap"
        ));
        assert!(!launchctl_says_unknown_subcommand(
            "Bootstrap failed: 5: Input/output error"
        ));
    }
}
