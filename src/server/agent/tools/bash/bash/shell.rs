//! How one admitted command line runs, stops and reports. The group it leads is the unit of stopping:
//! the backend supplies the program, `platform::proc` starts and signals the tree.

use std::sync::Arc;

use anyhow::{Context as _, Result};

use crate::server::agent::tools::contract::job::{Exit, OutputSink};
use crate::server::agent::tools::spec::ToolProgress;
use crate::server::platform::proc::{self, Signal};
use crate::server::platform::shell::Shell;
use crate::server::platform::stop::Stop;

/// One command line through the backend, collecting its output. The exit code and how it ended
/// live in the `Exit`; the text carries `[exit N]` and nothing about stopping.
pub(crate) fn run(
    shell: &Shell,
    cwd: &std::path::Path,
    cmd: &str,
    timeout: std::time::Duration,
    stop: &Arc<Stop>,
    progress: &mut dyn FnMut(ToolProgress),
) -> Result<(String, Exit)> {
    let mut sink = StringSink::default();
    let exit = run_to(shell, cwd, cmd, timeout, stop, progress, &mut sink)?;
    Ok((sink.text, exit))
}

/// Run one command line, feeding its two streams to `sink` as they arrive.
pub(crate) fn run_to(
    shell: &Shell,
    cwd: &std::path::Path,
    cmd: &str,
    timeout: std::time::Duration,
    stop: &Arc<Stop>,
    progress: &mut dyn FnMut(ToolProgress),
    sink: &mut dyn OutputSink,
) -> Result<Exit> {
    let flag = Arc::new(StopFlag::default());
    let mut command = shell.command(cmd);
    // A tool never reads the terminal: a prompt gets EOF, never a keystroke.
    command
        .current_dir(cwd)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let mut child = proc::spawn_grouped(&mut command)
        .with_context(|| format!("spawning `{}` failed", shell.name()))?;
    let pgid = child.id();

    // What stopping means here: TERM the group now, KILL it after the grace period if it lives.
    let escalate = {
        let flag = flag.clone();
        move || {
            if flag.armed.swap(true, std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            flag.ring();
            proc::signal_group(pgid, Signal::Term);
            let flag = flag.clone();
            named("mypi-bash-grace", move || {
                std::thread::sleep(proc::grace());
                if !flag.is_done() {
                    proc::signal_group(pgid, Signal::Kill);
                }
            });
        }
    };
    // Registered after the spawn: a stop that landed earlier runs this here and now.
    let hook = stop.on_stop(escalate.clone());

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
            let mut batch = String::new();
            /// How much one message carries: a chatty command must not cost one wake per line.
            const CHUNK: usize = 8 * 1024;
            loop {
                match rdr.read(&mut raw) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        // Lossy per read, keeping the trailing partial line back: a multi-byte character
                        // or escape split across reads must not be emitted half-formed.
                        pending.push_str(&String::from_utf8_lossy(&raw[..n]));
                        while let Some(i) = pending.find('\n') {
                            batch.extend(pending.drain(..=i));
                            if batch.len() >= CHUNK {
                                if tx.send(wrap(std::mem::take(&mut batch))).is_err() {
                                    return;
                                }
                            }
                        }
                    }
                }
            }
            if !pending.is_empty() {
                batch.push_str(&pending);
            }
            if !batch.is_empty() && tx.send(wrap(batch)).is_err() {
                return;
            }
            let _ = tx.send(Msg::ReaderDone);
        });
    }

    // Waiter thread: the only place blocked on the child's exit.
    let tx_wait = tx.clone();
    named("mypi-bash-wait", move || {
        let r = child.wait();
        let _ = tx_wait.send(Msg::Exited(r));
    });
    drop(tx);

    // Deadline: the same escalation, on the shared condvar instead of a sleeping thread.
    let timer = escalate.clone();
    let timer_flag = flag.clone();
    named("mypi-bash-deadline", move || {
        timer_flag.wait_for(timeout);
        if !timer_flag.is_done() {
            timer();
        }
    });

    // Tool thread: one blocking receive. Output goes to the front end as it arrives, so a command
    // that prints for a minute looks alive; nothing wakes this thread on a timer.
    /// How often the live view refreshes: fast enough to look live, slow enough that a chatty
    /// command cannot drown the front end.
    const REPORT_EVERY: std::time::Duration = std::time::Duration::from_millis(120);

    let mut status: Option<std::io::Result<std::process::ExitStatus>> = None;
    let mut readers_left = 2usize;
    let mut unflushed = String::new();
    let mut last_report = std::time::Instant::now();
    let mut sink_error: Option<anyhow::Error> = None;

    while readers_left > 0 || status.is_none() {
        match rx.recv() {
            Ok(Msg::Out(chunk)) => {
                unflushed.push_str(&chunk);
                if sink_error.is_none() {
                    if let Err(e) = sink.out(&chunk) {
                        sink_error = Some(e);
                        escalate();
                    }
                }
            }
            Ok(Msg::Err(chunk)) => {
                unflushed.push_str(&chunk);
                if sink_error.is_none() {
                    if let Err(e) = sink.err(&chunk) {
                        sink_error = Some(e);
                        escalate();
                    }
                }
            }
            Ok(Msg::ReaderDone) => readers_left = readers_left.saturating_sub(1),
            Ok(Msg::Exited(r)) => status = Some(r),
            Err(_) => anyhow::bail!("the shell waiter died unexpectedly"),
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
    // The command is over: take the action off the signal before the status, so an early return
    // below cannot signal a group that is already gone.
    drop(hook);
    flag.finish();
    let killed = flag.is_armed();
    let out = status
        .expect("loop only exits with a status")
        .with_context(|| "waiting for bash failed")?;
    let canceled = killed && stop.is_stopped();
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
        if exit.code != Some(0) && !exit.killed {
            text.push_str(&format!("\n[exit {}]", exit.code.unwrap_or(-1)));
        }
        let keep = text.trim_end().len();
        text.truncate(keep);
        self.text = text;
        Ok(())
    }
}

/// Why a bash command died without a clean exit. One flag, one bell: arming is one-way, and `done`
/// says the command is over so the deadline thread stops waiting.
#[derive(Default)]
struct StopFlag {
    armed: std::sync::atomic::AtomicBool,
    done: std::sync::atomic::AtomicBool,
    /// Wakes the deadline thread the instant either flag flips.
    bell: std::sync::Condvar,
    lock: std::sync::Mutex<()>,
}

impl StopFlag {
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
    /// Wake the waiter (arming and finishing both end the wait). The lock alone is not a signal:
    /// the waiter is parked on the condvar and needs `notify_all` to be scheduled.
    fn ring(&self) {
        drop(self.lock.lock().unwrap_or_else(|e| e.into_inner()));
        self.bell.notify_all();
    }
    /// Block until the flag is armed, finished, or `timeout` elapses. The deadline thread's only wait.
    fn wait_for(&self, timeout: std::time::Duration) {
        let guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self
            .bell
            .wait_timeout_while(guard, timeout, |_| !self.is_armed() && !self.is_done())
            .unwrap_or_else(|e| e.into_inner());
    }
}
