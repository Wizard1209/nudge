//! Let this process read and append to files iCloud has evicted.
//!
//! With "Optimise Mac Storage" on, iCloud Drive replaces a file it has
//! uploaded and not seen used with a *dataless* placeholder: the name and size
//! stay, the blocks go. Whether touching such a file downloads it or fails is a
//! per-process I/O policy, `IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES`, and the
//! process inherits it from whoever started it. Measured on macOS 27: an app
//! started through Launch Services (`open`) and a shell both get `ON`; a job
//! launchd starts from a LaunchAgent plist — which is how launch at login
//! starts Nudge — gets `OFF`, and under `OFF` the append to an evicted
//! journal fails with `EDEADLK` ("Resource deadlock avoided").
//!
//! The journal lives under `~/Documents`, which is iCloud Drive whenever
//! Desktop & Documents sync is on, so the policy is set once, process-wide, at
//! the top of `main`: what Nudge can open must not depend on how it was
//! started, and a sprite PNG kept in iCloud would fail the same way.

use std::ffi::c_int;

// `<sys/resource.h>`; the `libc` crate does not bind these.
unsafe extern "C" {
    fn setiopolicy_np(iotype: c_int, scope: c_int, policy: c_int) -> c_int;
    #[cfg(test)]
    fn getiopolicy_np(iotype: c_int, scope: c_int) -> c_int;
}
const IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES: c_int = 3;
const IOPOL_SCOPE_PROCESS: c_int = 0;
#[cfg(test)]
const IOPOL_MATERIALIZE_DATALESS_FILES_OFF: c_int = 1;
const IOPOL_MATERIALIZE_DATALESS_FILES_ON: c_int = 2;

/// Ask the kernel to download an evicted file when this process opens, reads or
/// writes it, instead of failing with `EDEADLK`. Inherited by the processes
/// this one spawns (the settings window).
pub fn allow_evicted_file_downloads() {
    let rc = unsafe {
        setiopolicy_np(
            IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
            IOPOL_SCOPE_PROCESS,
            IOPOL_MATERIALIZE_DATALESS_FILES_ON,
        )
    };
    if rc != 0 {
        eprintln!(
            "[nudge] cannot allow evicted iCloud files to download: {}",
            std::io::Error::last_os_error()
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy() -> c_int {
        unsafe {
            getiopolicy_np(
                IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
                IOPOL_SCOPE_PROCESS,
            )
        }
    }

    /// Starts from what launchd hands a LaunchAgent job, because `cargo test`
    /// inherits the shell's `ON` and would pass against a no-op.
    ///
    /// With `NUDGE_EVICTED_FILE` set to a file `brctl evict` has made dataless,
    /// it also appends to that file — the symptom itself, which needs iCloud and
    /// so cannot run unattended. Without it, only the policy is checked.
    #[test]
    fn a_launch_agent_start_can_append_to_an_evicted_journal() {
        let set = unsafe {
            setiopolicy_np(
                IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES,
                IOPOL_SCOPE_PROCESS,
                IOPOL_MATERIALIZE_DATALESS_FILES_OFF,
            )
        };
        assert_eq!(set, 0, "could not put the process in launchd's policy");
        assert_eq!(policy(), IOPOL_MATERIALIZE_DATALESS_FILES_OFF);

        allow_evicted_file_downloads();

        if let Some(path) = std::env::var_os("NUDGE_EVICTED_FILE") {
            use std::io::Write;
            let mut f = std::fs::OpenOptions::new()
                .append(true)
                .open(&path)
                .expect("open the evicted file");
            writeln!(f, "{{\"probe\":\"dataless\"}}").expect("append to the evicted file");
        }
        assert_eq!(policy(), IOPOL_MATERIALIZE_DATALESS_FILES_ON);
    }
}
