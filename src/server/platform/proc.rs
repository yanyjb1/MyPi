//! One command's process tree: started as its own group, ended by the group.

use std::process::{Child, Command};

#[cfg(unix)]
use std::time::{Duration, Instant};

/// How long a stopped tree gets between the polite signal and the fatal one.
pub fn grace() -> std::time::Duration {
    std::time::Duration::from_secs(2)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Signal {
    Term,
    Kill,
}

/// Spawn a command so that it leads a group of its own; every child it starts joins that group.
pub fn spawn_grouped(cmd: &mut Command) -> std::io::Result<Child> {
    sys::detach(cmd);
    cmd.spawn()
}

/// Kill this command's tree when the thread that forks it ends; Linux only.
#[cfg(target_os = "linux")]
pub fn guard_orphan(cmd: &mut Command) {
    use std::os::unix::process::CommandExt as _;
    // SAFETY: `prctl` is one syscall, and the child has not execed yet.
    unsafe {
        cmd.pre_exec(|| {
            if linux::parent_death_signal() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

/// Signal the whole group. A group that is already gone is the normal case.
pub fn signal_group(pgid: u32, signal: Signal) {
    sys::signal(pgid, signal);
}

/// Ask the tree to stop, then take it down; the leader is reaped before returning.
pub fn terminate_tree(child: &mut Child) {
    let pid = child.id();
    #[cfg(unix)]
    {
        signal_group(pid, Signal::Term);
        let deadline = Instant::now() + grace();
        loop {
            if matches!(child.try_wait(), Ok(Some(_))) {
                // A dead leader can leave stragglers behind; the group is the unit that must die.
                signal_group(pid, Signal::Kill);
                return;
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        signal_group(pid, Signal::Kill);
        let _ = child.wait();
    }
    #[cfg(windows)]
    {
        signal_group(pid, Signal::Kill);
        let _ = child.wait();
    }
}

#[cfg(unix)]
mod sys {
    use std::process::Command;

    unsafe extern "C" {
        fn setsid() -> i32;
        fn kill(pid: i32, sig: i32) -> i32;
    }

    const TERM: i32 = 15;
    const KILL: i32 = 9;

    /// A new session: no controlling terminal, and the child leads its own group.
    pub fn detach(cmd: &mut Command) {
        use std::os::unix::process::CommandExt as _;
        // SAFETY: `setsid` is one syscall and touches no shared state before the exec.
        unsafe {
            cmd.pre_exec(|| {
                if setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }

    pub fn signal(pgid: u32, signal: super::Signal) {
        let sig = match signal {
            super::Signal::Term => TERM,
            super::Signal::Kill => KILL,
        };
        // SAFETY: a plain syscall; a group that has already vanished is the normal case.
        unsafe {
            kill(-(pgid as i32), sig);
        }
    }
}

#[cfg(windows)]
mod sys {
    use std::process::Command;

    /// No console window for a child started from a process that has none.
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    pub fn detach(cmd: &mut Command) {
        use std::os::windows::process::CommandExt as _;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }

    pub fn signal(pid: u32, signal: super::Signal) {
        let mut cmd = Command::new("taskkill");
        cmd.args(["/T", "/PID", &pid.to_string()]);
        if signal == super::Signal::Kill {
            cmd.arg("/F");
        }
        let _ = cmd
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
}

#[cfg(target_os = "linux")]
mod linux {
    const PR_SET_PDEATHSIG: i32 = 1;
    const SIGKILL: i32 = 9;

    unsafe extern "C" {
        fn prctl(option: i32, ...) -> i32;
    }

    /// Ask the kernel to SIGKILL the child once the forking thread ends.
    pub fn parent_death_signal() -> i32 {
        // SAFETY: one syscall with constant arguments.
        unsafe { prctl(PR_SET_PDEATHSIG, SIGKILL as u64, 0u64, 0u64, 0u64) }
    }
}
