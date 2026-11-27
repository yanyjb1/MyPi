// One git invocation in a repository root's own frame: its exit, its stdout and its stderr, or nothing when git cannot start.

use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::time::Duration;

use crate::platform::proc::{Origin, Process};

// How long one git call may run before it and everything it forked are killed: a helper waiting on a prompt no terminal will answer must never hold the call it belongs to.
const LIMIT: Duration = Duration::from_secs(20);

// Run git with these arguments from `root`. `None` = git is not installed, or it and its children were killed at the deadline.
pub(super) fn git(root: &Path, args: &[&str]) -> Option<Output> {
    let mut cmd = Command::new("git");
    cmd.args(args)
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let process = Process::spawn(Origin::Tool, &mut cmd).ok()?;
    // Both pipes are drained on threads of their own: a child that fills one must never block on a reader that is not there yet.
    let out = drain(process.take_stdout()?);
    let err = drain(process.take_stderr()?);
    // The deadline passed with git still running: its whole tree is killed, and the call reads as git's silence.
    let Ok(Some(status)) = process.wait_timeout(LIMIT) else {
        process.stop();
        return None;
    };
    Some(Output {
        status,
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    })
}

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
