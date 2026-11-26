// One git invocation in a repository root's own frame: its exit, its stdout and its stderr, or nothing when git cannot start.

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

// How long one git call may run before it and everything it forked are killed: a helper waiting on a prompt no terminal will answer must never hold the call it belongs to.
const LIMIT: Duration = Duration::from_secs(20);

// How often the wait looks at the child: git answers in milliseconds, and a stalled one is killed at the deadline.
const TICK: Duration = Duration::from_millis(5);

const SIGKILL: i32 = 9;

#[cfg(unix)]
unsafe extern "C" {
    fn setsid() -> i32;
    fn kill(pid: i32, sig: i32) -> i32;
}

// Run git with these arguments from `root`. `None` = git is not installed, or it and its children were killed at the deadline.
pub(super) fn git(root: &Path, args: &[&str]) -> Option<Output> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    lead_own_group(&mut cmd);
    let mut child = cmd.spawn().ok()?;
    let group = child.id();
    // Both pipes are drained on threads of their own: a child that fills one must never block on a reader that is not there yet.
    let out = drain(child.stdout.take()?);
    let err = drain(child.stderr.take()?);
    let deadline = Instant::now() + LIMIT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) => {
                if Instant::now() >= deadline {
                    // The deadline passed with the child still running: it and its group are killed, and the call reads as git's silence.
                    let _ = child.kill();
                    kill_tree(group);
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(TICK);
            }
            Err(_) => {
                let _ = child.kill();
                kill_tree(group);
                let _ = child.wait();
                return None;
            }
        }
    };
    Some(Output {
        status,
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    })
}

// The child leads its own group, so a helper it forks dies with it instead of outliving the deadline.
#[cfg(unix)]
fn lead_own_group(cmd: &mut Command) {
    use std::os::unix::process::CommandExt as _;
    // SAFETY: `setsid` is one syscall, and the child has not execed yet.
    unsafe {
        cmd.pre_exec(|| {
            if setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

// Windows has no group to lead, and no session id to kill against: the child alone is killed at the deadline.
#[cfg(not(unix))]
fn lead_own_group(_cmd: &mut Command) {}

#[cfg(unix)]
fn kill_tree(group: u32) {
    // SAFETY: a plain syscall on a group that may already be gone, which it tolerates.
    unsafe {
        kill(-(group as i32), SIGKILL);
    }
}

#[cfg(not(unix))]
fn kill_tree(_group: u32) {}

// Read one pipe to its end on a thread of its own.
fn drain(pipe: impl std::io::Read + Send + 'static) -> std::thread::JoinHandle<Vec<u8>> {
    std::thread::Builder::new()
        .name("mypi-git-pipe".into())
        .spawn(move || {
            use std::io::Read as _;
            let mut rdr = std::io::BufReader::new(pipe);
            let mut text: Vec<u8> = Vec::new();
            let _ = rdr.read_to_end(&mut text);
            text
        })
        .expect("a reader thread")
}

// What git said on its error stream, cut to one line: the sentence a tool hands back.
pub(super) fn why(out: &Output) -> String {
    let text = String::from_utf8_lossy(&out.stderr);
    text.lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or("git failed")
        .to_string()
}
