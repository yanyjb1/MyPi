//! How one shell command runs, stops and reports. Process-group ownership lives here, not in
//! coreutils `timeout`: `setsid --wait bash -c`, one kill for the whole group, TERM then KILL.

use anyhow::{Context as _, Result};

use crate::server::agent::tools::spec::ToolProgress;

/// Where one command's output goes while it runs.
pub(crate) trait OutputSink {
    fn out(&mut self, chunk: &str) -> Result<()>;
    fn err(&mut self, chunk: &str) -> Result<()>;
    fn done(&mut self, exit: &Exit) -> Result<()>;
}

/// Why a command stopped. `canceled` means an external stop armed the flag before the deadline.
pub(crate) struct Exit {
    pub code: Option<i32>,
    pub killed: bool,
    pub canceled: bool,
    pub timeout_secs: u64,
}

/// `bash -c` one command and collect its output. The caller checked there are no newlines. Exit code
/// and interrupt both live in the text (`[exit N]`, `[interrupted/timeout: …]`).
pub(crate) fn run(
    cwd: &std::path::Path,
    cmd: &str,
    timeout: std::time::Duration,
    external: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    progress: &mut dyn FnMut(ToolProgress),
) -> Result<String> {
    let mut sink = StringSink::default();
    run_to(cwd, cmd, timeout, external, progress, &mut sink)?;
    Ok(sink.text)
}

/// Run one command, feeding its two streams to `sink` as they arrive.
pub(crate) fn run_to(
    cwd: &std::path::Path,
    cmd: &str,
    timeout: std::time::Duration,
    external: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    progress: &mut dyn FnMut(ToolProgress),
    sink: &mut dyn OutputSink,
) -> Result<Exit> {
    let stop = std::sync::Arc::new(StopFlag::default());
    let external_stop = external.clone();
    if let Some(ext) = external {
        // Bridge the session's AtomicBool into this command's flag. Exit on `is_done`, **not**
        // `is_armed`: a command nobody interrupts never arms the flag.
        let bridged = stop.clone();
        named("mypi-bash-bridge", move || {
            // Wake on the flag (a condvar, not a 50ms poll), and end on `is_done`, so a plain
            // successful call leaves no poller spinning behind.
            while !bridged.is_armed() && !bridged.is_done() {
                if ext.load(std::sync::atomic::Ordering::Relaxed) {
                    bridged.arm();
                    break;
                }
                bridged.wait_for(std::time::Duration::from_millis(20));
            }
        });
    }
    let mut child = std::process::Command::new("setsid")
        .arg("--wait")
        .arg("bash")
        .arg("-c")
        .arg(cmd)
        .current_dir(cwd)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .with_context(|| "spawn bash failed (util-linux setsid missing?)")?;
    let pgid = child.id();

    // Everything about this command travels on one channel: both output streams and the exit status.
    // Line-oriented on purpose — the readers reassemble whole lines before saying anything.
    enum Msg {
        Out(String),
        Err(String),
        ReaderDone,
        Exited(std::io::Result<std::process::ExitStatus>),
    }
    let (tx, rx) = std::sync::mpsc::channel::<Msg>();

    let stdout = child.stdout.take().expect("stdout is piped");
    let stderr = child.stderr.take().expect("stderr is piped");
    for (pipe, wrap) in [
        (
            Box::new(stdout) as Box<dyn std::io::Read + Send>,
            Msg::Out as fn(String) -> Msg,
        ),
        (
            Box::new(stderr) as Box<dyn std::io::Read + Send>,
            Msg::Err as fn(String) -> Msg,
        ),
    ] {
        let tx = tx.clone();
        named("mypi-bash-read", move || {
            use std::io::Read as _;
            let mut rdr = std::io::BufReader::new(pipe);
            let mut raw = [0u8; 8192];
            let mut pending = String::new();
            loop {
                match rdr.read(&mut raw) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        // Lossy per read, keeping the trailing partial line back: a multi-byte character
                        // or escape split across reads must not be emitted half-formed.
                        pending.push_str(&String::from_utf8_lossy(&raw[..n]));
                        while let Some(i) = pending.find('\n') {
                            let line: String = pending.drain(..=i).collect();
                            if tx.send(wrap(line)).is_err() {
                                return;
                            }
                        }
                    }
                }
            }
            if !pending.is_empty() {
                let _ = tx.send(wrap(pending));
            }
            let _ = tx.send(Msg::ReaderDone);
        });
    }

    // Waiter thread: the only place blocked on the child's exit.
    let waiter_stop = stop.clone();
    let tx_wait = tx.clone();
    named("mypi-bash-wait", move || {
        let r = child.wait();
        let _ = tx_wait.send(Msg::Exited(r));
        drop(waiter_stop);
    });
    drop(tx);

    // Deadline: arms the same flag the front-end stop arms, waiting on the shared condvar instead of
    // sleeping out the timeout.
    let timer_stop = stop.clone();
    named("mypi-bash-deadline", move || {
        timer_stop.wait_for(timeout);
        timer_stop.arm();
    });

    // Tool thread: drain with ticks; on stop, escalate TERM then KILL. Output goes to the front end as
    // it arrives, so a command that prints for a minute looks alive.
    const TICK: std::time::Duration = std::time::Duration::from_millis(50);
    /// How often the live view refreshes: fast enough to look live, slow enough that a chatty
    /// command cannot drown the front end.
    const REPORT_EVERY: std::time::Duration = std::time::Duration::from_millis(120);

    let mut status: Option<std::io::Result<std::process::ExitStatus>> = None;
    let mut readers_left = 2usize;
    let mut unflushed = String::new();
    let mut last_report = std::time::Instant::now();
    let mut killed = false;
    let mut sink_error: Option<anyhow::Error> = None;

    while readers_left > 0 || status.is_none() {
        match rx.recv_timeout(TICK) {
            Ok(Msg::Out(chunk)) => {
                unflushed.push_str(&chunk);
                if sink_error.is_none() {
                    if let Err(e) = sink.out(&chunk) {
                        sink_error = Some(e);
                        stop.arm();
                    }
                }
            }
            Ok(Msg::Err(chunk)) => {
                unflushed.push_str(&chunk);
                if sink_error.is_none() {
                    if let Err(e) = sink.err(&chunk) {
                        sink_error = Some(e);
                        stop.arm();
                    }
                }
            }
            Ok(Msg::ReaderDone) => readers_left = readers_left.saturating_sub(1),
            Ok(Msg::Exited(r)) => status = Some(r),
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                anyhow::bail!("bash waiter died unexpectedly")
            }
        }
        if stop.is_armed() && !killed {
            killed = true;
            group_signal(pgid, "TERM");
        }
        if !unflushed.is_empty() && last_report.elapsed() >= REPORT_EVERY {
            last_report = std::time::Instant::now();
            progress(ToolProgress::Output(crate::ansi::strip_ansi(&unflushed)));
            unflushed.clear();
        }
    }
    if !unflushed.is_empty() {
        progress(ToolProgress::Output(crate::ansi::strip_ansi(&unflushed)));
    }
    // The command is over: release the interrupt bridge before touching the status, so the early
    // returns below cannot strand a polling thread.
    stop.finish();
    let killed = killed || stop.is_armed();
    let out = status
        .expect("loop only exits with a status")
        .with_context(|| "waiting for bash failed")?;
    let canceled = killed
        && external_stop
            .as_ref()
            .is_some_and(|e| e.load(std::sync::atomic::Ordering::Relaxed));
    let exit = Exit {
        code: out.code(),
        killed,
        canceled,
        timeout_secs: timeout.as_secs().max(1),
    };
    if let Some(e) = sink_error {
        return Err(e);
    }
    sink.done(&exit)?;
    Ok(exit)
}

/// Spawn with a name: these threads live as long as one bash command, and an unnamed pool is
/// unreadable in `top -H`. A name that fails is not worth failing the command over.
fn named(name: &str, f: impl FnOnce() + Send + 'static) {
    let _ = std::thread::Builder::new().name(name.into()).spawn(f);
}

/// The bash tool's original string result, byte for byte.
#[derive(Default)]
struct StringSink {
    stdout: String,
    stderr: String,
    text: String,
}

impl OutputSink for StringSink {
    fn out(&mut self, chunk: &str) -> Result<()> {
        self.stdout.push_str(chunk);
        Ok(())
    }

    fn err(&mut self, chunk: &str) -> Result<()> {
        self.stderr.push_str(chunk);
        Ok(())
    }

    fn done(&mut self, exit: &Exit) -> Result<()> {
        let stdout = crate::ansi::strip_ansi(&self.stdout);
        let err = crate::ansi::strip_ansi(&self.stderr);
        let mut text = stdout;
        if !err.trim().is_empty() {
            text.push_str("\n[stderr] ");
            text.push_str(err.trim_end());
        }
        if exit.killed {
            text.push_str(&format!(
                "\n[interrupted/timeout: killed after {}s]",
                exit.timeout_secs
            ));
        } else if exit.code != Some(0) {
            text.push_str(&format!("\n[exit {}]", exit.code.unwrap_or(-1)));
        }
        self.text = text.trim_end().to_string();
        Ok(())
    }
}

/// Why a bash command died without a clean exit. Shared with the waiter and deadline threads; arming
/// is one-way, and `done` says the command is over, which is what ends the bridge's wait.
#[derive(Default)]
struct StopFlag {
    armed: std::sync::atomic::AtomicBool,
    done: std::sync::atomic::AtomicBool,
    /// Wakes both waiters the instant either flag flips, so neither has to
    /// poll (the bridge) or sleep out a long timeout (the deadline).
    bell: std::sync::Condvar,
    lock: std::sync::Mutex<()>,
}

impl StopFlag {
    fn arm(&self) {
        self.armed.store(true, std::sync::atomic::Ordering::Relaxed);
        self.ring();
    }
    fn is_armed(&self) -> bool {
        self.armed.load(std::sync::atomic::Ordering::Relaxed)
    }
    /// Command finished (however it finished): stop every thread waiting on this flag. Idempotent.
    fn finish(&self) {
        self.done.store(true, std::sync::atomic::Ordering::Relaxed);
        self.ring();
    }
    fn is_done(&self) -> bool {
        self.done.load(std::sync::atomic::Ordering::Relaxed)
    }
    /// Wake every waiter (arming and finishing both end the wait). The lock alone is not a signal:
    /// the waiters are parked on the condvar and need `notify_all` to be scheduled.
    fn ring(&self) {
        drop(self.lock.lock().unwrap_or_else(|e| e.into_inner()));
        self.bell.notify_all();
    }
    /// Block until the flag is armed, finished, or `timeout` elapses. Used by the deadline thread in
    /// place of a bare `sleep`, and by the bridge in place of a poll loop.
    fn wait_for(&self, timeout: std::time::Duration) {
        let guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self
            .bell
            .wait_timeout_while(guard, timeout, |_| !self.is_armed() && !self.is_done())
            .unwrap_or_else(|e| e.into_inner());
    }
}

/// Signal a whole process group via coreutils `kill` (negative pid = group); when `kill` is missing,
/// the SIGKILL fallback on the direct child still lands via the waiter.
fn group_signal(pgid: u32, sig: &str) {
    let _ = std::process::Command::new("kill")
        .args([format!("-{sig}"), format!("-{pgid}")])
        .status();
}
