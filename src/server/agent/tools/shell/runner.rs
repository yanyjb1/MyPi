// How one admitted command line runs, stops and reports. The group it leads is the unit of stopping: the backend supplies the program, `platform::proc` starts and signals the tree.

use std::sync::Arc;

use anyhow::{Context as _, Result};

use crate::server::agent::tools::contract::job::{Exit, OutputSink};
use crate::server::agent::tools::spec::ToolProgress;
use crate::server::platform::proc;
use crate::server::platform::shell::Shell;
use crate::server::platform::stop::Stop;

// One command line through the backend, collecting its output. The exit code and how it ended live in the `Exit`; the text carries `[exit N]` and nothing about stopping.
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

// Run one command line, feeding its two streams to `sink` as they arrive.
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
    let process = proc::Process::spawn(proc::Origin::Tool, &mut command)
        .with_context(|| format!("spawning `{}` failed", shell.name()))?;

    // What stopping means here: one call to the platform, which owns the ladder.
    let escalate = {
        let flag = flag.clone();
        let process = process.clone();
        move || {
            if flag.armed.swap(true, std::sync::atomic::Ordering::Relaxed) {
                return;
            }
            flag.ring();
            process.stop();
        }
    };
    // Registered after the spawn: a stop that landed earlier runs this here and now.
    let hook = stop.on_stop(escalate.clone());

    // Everything about this command travels on one channel: both output streams and the exit status. A reader hands on every byte it can already be read as text, so a command that prints once a second reaches the live view once a second.
    enum Msg {
        Out(String),
        Err(String),
        ReaderDone,
        Exited(std::io::Result<std::process::ExitStatus>),
    }
    let (tx, rx) = std::sync::mpsc::channel::<Msg>();

    let stdout = process.take_stdout().expect("stdout is piped");
    let stderr = process.take_stderr().expect("stderr is piped");
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
        named("mypi-shell-read", move || {
            use std::io::Read as _;
            let mut rdr = std::io::BufReader::new(pipe);
            let mut raw = [0u8; 8192];
            let mut pending: Vec<u8> = Vec::new();
            loop {
                match rdr.read(&mut raw) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        // Whatever this read brought goes on unless its tail is still unreadable: half a character, half an escape sequence, half a `\r\n`.
                        pending.extend_from_slice(&raw[..n]);
                        let at = readable(&pending);
                        if at == 0 {
                            continue;
                        }
                        let text = plain(&pending[..at]);
                        pending.drain(..at);
                        if tx.send(wrap(text)).is_err() {
                            return;
                        }
                    }
                }
            }
            if !pending.is_empty() && tx.send(wrap(plain(&pending))).is_err() {
                return;
            }
            let _ = tx.send(Msg::ReaderDone);
        });
    }

    // Waiter thread: the only place blocked on the child's exit.
    let tx_wait = tx.clone();
    let waiter = process.clone();
    named("mypi-shell-wait", move || {
        let r = waiter.wait();
        let _ = tx_wait.send(Msg::Exited(r));
    });
    drop(tx);

    // Deadline: the same escalation, on the shared condvar instead of a sleeping thread.
    let timer = escalate.clone();
    let timer_flag = flag.clone();
    named("mypi-shell-deadline", move || {
        timer_flag.wait_for(timeout);
        if !timer_flag.is_done() {
            timer();
        }
    });

    // Tool thread: one blocking receive, and every message goes on the moment it lands. The pipe is the only clock: a read returns as soon as the command writes.
    let mut status: Option<std::io::Result<std::process::ExitStatus>> = None;
    let mut readers_left = 2usize;
    let mut sink_error: Option<anyhow::Error> = None;

    while readers_left > 0 || status.is_none() {
        match rx.recv() {
            Ok(Msg::Out(chunk)) => {
                if sink_error.is_none()
                    && let Err(e) = sink.out(&chunk)
                {
                    sink_error = Some(e);
                    escalate();
                }
                progress(ToolProgress::Output(crate::ansi::strip_ansi(&chunk)));
            }
            Ok(Msg::Err(chunk)) => {
                if sink_error.is_none()
                    && let Err(e) = sink.err(&chunk)
                {
                    sink_error = Some(e);
                    escalate();
                }
                progress(ToolProgress::Output(crate::ansi::strip_ansi(&chunk)));
            }
            Ok(Msg::ReaderDone) => readers_left = readers_left.saturating_sub(1),
            Ok(Msg::Exited(r)) => status = Some(r),
            Err(_) => anyhow::bail!("the shell waiter died unexpectedly"),
        }
    }
    // The command is over: take the action off the signal before the status, so an early return below cannot signal a group that is already gone.
    drop(hook);
    flag.finish();
    let killed = flag.is_armed();
    let out = status
        .expect("loop only exits with a status")
        .with_context(|| "waiting for the shell failed")?;
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

// Spawn with a name: these threads live as long as one shell command, and an unnamed pool is unreadable in `top -H`. A name that fails is not worth failing the command over.
fn named(name: &str, f: impl FnOnce() + Send + 'static) {
    let _ = std::thread::Builder::new().name(name.into()).spawn(f);
}

// The shell tool's original string result, byte for byte.
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

// Why a shell command died without a clean exit. One flag, one bell: arming is one-way, and `done` says the command is over so the deadline thread stops waiting.
#[derive(Default)]
struct StopFlag {
    armed: std::sync::atomic::AtomicBool,
    done: std::sync::atomic::AtomicBool,
    // Wakes the deadline thread the instant either flag flips.
    bell: std::sync::Condvar,
    lock: std::sync::Mutex<()>,
}

impl StopFlag {
    fn is_armed(&self) -> bool {
        self.armed.load(std::sync::atomic::Ordering::Relaxed)
    }
    // Command finished (however it finished): stop every thread waiting on this flag. Idempotent.
    fn finish(&self) {
        self.done.store(true, std::sync::atomic::Ordering::Relaxed);
        self.ring();
    }
    fn is_done(&self) -> bool {
        self.done.load(std::sync::atomic::Ordering::Relaxed)
    }
    // Wake the waiter (arming and finishing both end the wait). The lock alone is not a signal: the waiter is parked on the condvar and needs `notify_all` to be scheduled.
    fn ring(&self) {
        drop(self.lock.lock().unwrap_or_else(|e| e.into_inner()));
        self.bell.notify_all();
    }
    // Block until the flag is armed, finished, or `timeout` elapses. The deadline thread's only wait.
    fn wait_for(&self, timeout: std::time::Duration) {
        let guard = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let _ = self
            .bell
            .wait_timeout_while(guard, timeout, |_| !self.is_armed() && !self.is_done())
            .unwrap_or_else(|e| e.into_inner());
    }
}

// Where the bytes stop being text this call may hand on: an unfinished trailing character or escape sequence stays for the next read.
fn readable(bytes: &[u8]) -> usize {
    let mut at = utf8_end(bytes);
    if let Some(open) = open_escape(&bytes[..at]) {
        at = open;
    }
    // A trailing `\r` may be the first half of a `\r\n`.
    if at > 0 && bytes[at - 1] == b'\r' {
        at -= 1;
    }
    at
}

// Where an unfinished trailing UTF-8 character starts; its lead byte sits in the last four.
fn utf8_end(bytes: &[u8]) -> usize {
    let len = bytes.len();
    let floor = len.saturating_sub(4);
    let mut at = len;
    while at > floor {
        at -= 1;
        let byte = bytes[at];
        if byte & 0xC0 == 0x80 {
            continue;
        }
        let need = match byte {
            0x00..=0x7F => 1,
            0xC0..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF7 => 4,
            _ => return len,
        };
        return if len - at < need { at } else { len };
    }
    len
}

// Where an unfinished trailing escape sequence starts: text is handed on in whole sequences, so `ansi::strip_ansi` never sees half of one.
fn open_escape(bytes: &[u8]) -> Option<usize> {
    let from = bytes.iter().rposition(|byte| *byte == 0x1b)?;
    (!escape_done(&bytes[from..])).then_some(from)
}

// Whether the sequence that starts at this ESC has ended.
fn escape_done(tail: &[u8]) -> bool {
    match tail.get(1) {
        None => false,
        Some(b'[') => tail[2..].iter().any(|byte| (0x40..=0x7e).contains(byte)),
        Some(b']') => {
            let body = &tail[2..];
            let mut at = 0;
            while at < body.len() {
                if body[at] == 0x07 {
                    return true;
                }
                if body[at] == 0x1b {
                    return at + 1 >= body.len() || body[at + 1] == b'\\';
                }
                at += 1;
            }
            false
        }
        Some(0x20..=0x2f) => tail[2..].iter().any(|byte| !(0x20..=0x2f).contains(byte)),
        Some(_) => true,
    }
}

// The bytes as text, every `\r\n` folded to `\n`; a lone `\r` stays, because a progress bar returns to column zero with it.
fn plain(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\r' && chars.peek() == Some(&'\n') {
            continue;
        }
        out.push(c);
    }
    out
}
