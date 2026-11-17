use crate::config::{Config, ConfigError};

pub trait AutostartProvider {
    fn enable(&self) -> Result<(), String>;
    fn disable(&self) -> Result<(), String>;
    fn is_enabled(&self) -> Result<bool, String>;
    fn location(&self) -> String {
        "unspecified".to_string()
    }
}

pub fn provider_for_current_exe(
    #[cfg_attr(
        any(target_os = "windows", target_os = "macos"),
        allow(unused_variables)
    )]
    initial_enabled: bool,
) -> Result<Box<dyn AutostartProvider>, String> {
    #[cfg(target_os = "windows")]
    {
        WindowsRegistryProvider::for_current_exe()
            .map(|p| Box::new(p) as Box<dyn AutostartProvider>)
    }
    #[cfg(target_os = "macos")]
    {
        MacLaunchAgentProvider::for_current_exe().map(|p| Box::new(p) as Box<dyn AutostartProvider>)
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        Ok(Box::new(FakeProvider::new(initial_enabled)))
    }
}

#[derive(Debug)]
pub enum AutostartError {
    Backend(String),
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

#[cfg(any(test, not(any(target_os = "windows", target_os = "macos"))))]
pub struct FakeProvider {
    enabled: std::cell::Cell<bool>,
    fail_enable: bool,
    fail_disable: bool,
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
    fn location(&self) -> String {
        "in-memory fake".to_string()
    }
}

#[cfg(target_os = "windows")]
pub const RUN_VALUE_NAME: &str = "Nudge";

#[cfg(target_os = "windows")]
pub const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

#[cfg(target_os = "windows")]
pub struct WindowsRegistryProvider {
    exe: std::path::PathBuf,
}

#[cfg(target_os = "windows")]
impl WindowsRegistryProvider {
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
    fn location(&self) -> String {
        format!("HKCU\\{RUN_SUBKEY} value \"{RUN_VALUE_NAME}\"")
    }
    fn enable(&self) -> Result<(), String> {
        use windows::Win32::Foundation::ERROR_SUCCESS;
        use windows::Win32::System::Registry::{
            HKEY, HKEY_CURRENT_USER, KEY_SET_VALUE, REG_SZ, RegCloseKey, RegOpenKeyExW,
            RegSetValueExW,
        };
        use windows::core::PCWSTR;

        let subkey = to_wide(RUN_SUBKEY);
        let name = to_wide(RUN_VALUE_NAME);
        // The Run value's data is a command line, so a path with spaces is quoted.
        // proof: https://learn.microsoft.com/en-us/windows/win32/setupapi/run-and-runonce-registry-keys
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

#[cfg(target_os = "macos")]
pub const LAUNCH_AGENT_LABEL: &str = "com.wizard1209.nudge";

#[cfg(target_os = "macos")]
pub struct MacLaunchAgentProvider {
    program: std::path::PathBuf,
    plist_path: std::path::PathBuf,
}

#[cfg(target_os = "macos")]
impl MacLaunchAgentProvider {
    pub fn for_current_exe() -> Result<Self, String> {
        let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;
        let program = std::fs::canonicalize(&exe).unwrap_or(exe);

        let home = dirs::home_dir().ok_or_else(|| "cannot resolve home directory".to_string())?;
        Ok(Self::in_home(&home, program))
    }

    pub fn in_home(home: &std::path::Path, program: std::path::PathBuf) -> Self {
        Self {
            plist_path: launch_agent_plist_path(home, LAUNCH_AGENT_LABEL),
            program,
        }
    }

    pub fn plist_path(&self) -> &std::path::Path {
        &self.plist_path
    }

    pub fn program(&self) -> &std::path::Path {
        &self.program
    }

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

    pub fn remove_plist(&self) -> Result<bool, String> {
        match std::fs::remove_file(&self.plist_path) {
            Ok(()) => Ok(true),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
            Err(e) => Err(format!("remove {}: {e}", self.plist_path.display())),
        }
    }

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

#[cfg(target_os = "macos")]
pub fn launch_agent_plist_path(home: &std::path::Path, label: &str) -> std::path::PathBuf {
    home.join("Library/LaunchAgents")
        .join(format!("{label}.plist"))
}

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

#[cfg(target_os = "macos")]
pub fn parse_plist_label(xml: &str) -> Option<String> {
    let after_key = xml.split_once("<key>Label</key>")?.1;
    let open = after_key.find("<string>")? + "<string>".len();
    let close = after_key[open..].find("</string>")? + open;
    Some(after_key[open..close].trim().to_string())
}

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

#[cfg(target_os = "macos")]
fn launchctl_says_already_registered(code: Option<i32>, text: &str) -> bool {
    matches!(code, Some(17) | Some(37))
        || text.contains("already bootstrapped")
        || text.contains("already loaded")
        || text.contains("Already loaded")
}

#[cfg(target_os = "macos")]
fn launchctl_says_not_registered(code: Option<i32>, text: &str) -> bool {
    matches!(code, Some(3) | Some(113))
        || text.contains("Could not find specified service")
        || text.contains("No such process")
        || text.contains("not find service")
}

#[cfg(target_os = "macos")]
fn launchctl_says_unknown_subcommand(text: &str) -> bool {
    text.contains("Unrecognized subcommand") || text.contains("Unknown subcommand")
}

#[cfg(target_os = "macos")]
fn current_uid() -> u32 {
    unsafe extern "C" {
        fn getuid() -> u32;
    }
    unsafe { getuid() }
}

#[cfg(target_os = "macos")]
impl AutostartProvider for MacLaunchAgentProvider {
    fn location(&self) -> String {
        format!(
            "LaunchAgent {LAUNCH_AGENT_LABEL}: plist {}, program {}",
            self.plist_path.display(),
            self.program.display()
        )
    }
    fn enable(&self) -> Result<(), String> {
        self.write_plist()?;
        if let Err(e) = self.register() {
            let _ = self.remove_plist();
            return Err(e);
        }
        Ok(())
    }

    fn disable(&self) -> Result<(), String> {
        let _ = self.unregister();
        self.remove_plist()?;
        Ok(())
    }

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
        let provider = FakeProvider::new(true);
        let mut config = Config::default();
        let saved = std::cell::RefCell::new(None);

        apply_autostart(&provider, &mut config, true, recording_persist(&saved)).unwrap();

        assert!(config.autostart);
        assert_eq!(saved.borrow().as_ref().map(|c| c.autostart), Some(true));
    }

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
