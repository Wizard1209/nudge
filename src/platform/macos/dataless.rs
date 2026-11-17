// A process inherits IOPOL_TYPE_VFS_MATERIALIZE_DATALESS_FILES from its parent, and the
// system default for the process scope is OFF, which disables materialization of dataless files.
// proof: https://github.com/apple-oss-distributions/xnu/blob/ac9718f/bsd/man/man3/getiopolicy_np.3

use std::ffi::c_int;

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
