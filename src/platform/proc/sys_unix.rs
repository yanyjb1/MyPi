// Unix container: one session per tree, group signals, and the three-step stop.

use std::io;
use std::process::Command;

use super::{Follow, GRACE, Grace, Inner, KILL_LIMIT, Phase, Process, lock};

unsafe extern "C" {
    fn setsid() -> i32;
    fn kill(pid: i32, sig: i32) -> i32;
}

const SIGTERM: i32 = 15;
const SIGKILL: i32 = 9;
const EPERM: i32 = 1;

pub const CONTAINER: &str = "group";

// A new session: no controlling terminal, and the child leads its own group.
pub fn prepare(cmd: &mut Command, follow: Follow) {
    use std::os::unix::process::CommandExt as _;
    // SAFETY: `setsid` is one syscall, and the child has not execed yet.
    unsafe {
        cmd.pre_exec(move || {
            if setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            #[cfg(any(target_os = "linux", target_os = "android"))]
            if follow == Follow::Main && linux::die_with_parent() == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

pub struct Group {
    pgid: u32,
}

impl Group {
    pub fn attach(pid: u32, _follow: Follow) -> Group {
        Group { pgid: pid }
    }

    pub fn backed_off(&self) -> bool {
        false
    }

    pub fn request_stop(&self) {
        // SAFETY: a plain syscall; a group that has already vanished is the normal case.
        unsafe {
            kill(-(self.pgid as i32), SIGTERM);
        }
    }

    pub fn terminate(&self) {
        // SAFETY: a plain syscall; a group that has already vanished is the normal case.
        unsafe {
            kill(-(self.pgid as i32), SIGKILL);
        }
    }

    // Whether anything was left in the group; leftovers are killed here.
    pub fn reclaim(&self) -> bool {
        // SAFETY: signal 0 only probes the group; both calls tolerate a vanished group.
        let alive = unsafe { kill(-(self.pgid as i32), 0) } == 0
            || io::Error::last_os_error().raw_os_error() == Some(EPERM);
        if !alive {
            return false;
        }
        self.terminate();
        true
    }
}

// Unix needs no second step: the fork thread finishes the child's setup before it execs.
pub fn resume(_pid: u32) -> Result<(), String> {
    Ok(())
}

// Unix has no handle to close: the group disappears with its members.
pub fn stop(process: &Process) {
    let inner: &Inner = &process.0;
    if inner.exit.published() {
        return;
    }
    {
        let mut phase = lock(&inner.stop_phase);
        if inner.exit.published() || phase.phase != Phase::None {
            return;
        }
        phase.phase = Phase::Requesting;
        // Written before the wait so the reaper never records "none" for a polite stop.
        phase.grace = Grace::Within;
    }
    inner.group.request_stop();
    let within = match inner.exit.wait_timeout(GRACE) {
        Ok(Some(_)) | Err(_) => true,
        Ok(None) => false,
    };
    {
        let mut phase = lock(&inner.stop_phase);
        phase.grace = if within {
            Grace::Within
        } else {
            Grace::Ignored
        };
        phase.phase = Phase::Terminating;
    }
    if !within {
        inner.group.terminate();
        let _ = inner.exit.wait_timeout(KILL_LIMIT);
    }
}

#[cfg(any(target_os = "linux", target_os = "android"))]
mod linux {
    const PR_SET_PDEATHSIG: i32 = 1;
    const SIGKILL: i32 = 9;

    unsafe extern "C" {
        fn prctl(option: i32, ...) -> i32;
    }

    // Ask the kernel to kill the leader once the forking thread ends.
    pub fn die_with_parent() -> i32 {
        // SAFETY: one syscall with constant arguments.
        unsafe { prctl(PR_SET_PDEATHSIG, SIGKILL as u64, 0u64, 0u64, 0u64) }
    }
}
