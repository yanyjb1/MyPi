//! Built-in tools: edit, mass_edit, bash, cd.
//!
//! Mirrors the shape of pi's packages/coding-agent/src/core/tools/ (one
//! schema per tool, registered into a registry), but lives in a single
//! module while the tool set is small: each tool is a "parse + execute"
//! pair.
//!
//! # Why two text-replacement tools
//!
//! **edit** finds `old` anywhere in the file and replaces it with `new`.
//! It refuses to run unless `old` occurs exactly once — a "replace_all"
//! knob is convenient, but models tend to sweep up places that should not
//! change, and a failed run is hard to localize afterwards.
//!
//! **mass_edit** replaces whole lines by number. It never searches.
//!
//! Its use case is exactly where edit hits the wall: replacing the same
//! word on many lines that look identical makes edit fail with
//! "ambiguous match", while mass_edit just names the lines.
//!
//! # Argument parsing
//!
//! `arguments` arrives as a raw string and is not trusted: fields may be
//! missing, values may be quoted, paths may be `./x` or `../x`. Every
//! tool therefore starts with parse + validate and returns a descriptive
//! error — the error goes back to the model, which self-corrects.

use anyhow::{Context as _, Result, anyhow};

use crate::server::ai::types::{ToolCall, ToolDef};

use super::loop_rs::{ToolOutput, ToolProgress};

// Canonicalize as far as the path exists, then re-append the tail.
//
// Needed because a `write` target may not exist yet, and because the zone check
// must run on the *real* path: a lexical check (`/tmp/link/../../etc`) would
// pass while the kernel resolves somewhere else entirely.
fn canonicalize_lenient(p: &std::path::Path) -> std::path::PathBuf {
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = p.to_path_buf();
    loop {
        if let Ok(real) = cur.canonicalize() {
            let mut out = real;
            for part in tail.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (cur.parent().map(|x| x.to_path_buf()), cur.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name.to_os_string());
                cur = parent;
            }
            _ => return p.to_path_buf(),
        }
    }
}

/// The zones a **file tool** may touch: the session workspace, plus the system
/// temp dir.
///
/// Temp is licensed because scratch work lives there (`git clone /tmp/…`,
/// artifact materializations) — and `bash_guard` already licenses the same two
/// for deletes. One rule, two enforcers; a second, drifted notion of "inside"
/// is exactly what this function exists to prevent.
pub fn licensed_zones(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut zones = vec![canonicalize_lenient(root)];
    let tmp = std::env::temp_dir();
    zones.push(canonicalize_lenient(&tmp));
    zones.dedup();
    zones
}

/// Resolve a model-supplied path for a file tool.
///
/// Relative paths resolve against the session `cwd`; `..` and symlinks are
/// digested *before* the zone check; the result must land in a licensed zone.
fn resolve_path(
    cwd: &std::path::Path,
    zones: &[std::path::PathBuf],
    raw: &str,
    must_exist: bool,
) -> Result<std::path::PathBuf> {
    anyhow::ensure!(!raw.trim().is_empty(), "empty path");
    let p = std::path::Path::new(raw);
    let p = if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    };
    let real = canonicalize_lenient(&p);
    if !zones.iter().any(|z| real.starts_with(z)) {
        anyhow::bail!(
            "denied: {} is outside the workspace ({}) and the temp dir — \
             文件工具只能读写这两处",
            real.display(),
            zones
                .first()
                .map(|z| z.display().to_string())
                .unwrap_or_default()
        );
    }
    if must_exist {
        if !real.exists() {
            return Err(anyhow!("file not found: {}", real.display()));
        }
        if real.is_dir() {
            return Err(anyhow!(
                "{} is a directory; only files can be read or edited",
                real.display()
            ));
        }
    }
    Ok(real)
}

// Fetch a string field from the arguments; a missing field is a readable error.
fn need_str(args: &serde_json::Value, key: &str) -> Result<String> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| anyhow!("missing or non-string argument `{key}`"))
}

// Fetch a positive integer line number (1-based) from the arguments.
fn need_line(args: &serde_json::Value, key: &str) -> Result<usize> {
    let n = args
        .get(key)
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow!("missing or non-numeric argument `{key}`"))?;
    if n == 0 {
        return Err(anyhow!("line numbers start at 1, got 0"));
    }
    Ok(n as usize)
}

// ---------------------------------------------------------------------------
// edit
// ---------------------------------------------------------------------------

// edit arguments.
#[derive(Debug)]
pub struct EditArgs {
    pub path: String,
    pub old: String,
    pub new: String,
}

// ---------------------------------------------------------------------------
/// Arguments for `read`: a file, and where to start in it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadArgs {
    pub path: String,
    /// First line to read, 1-based (the same numbering the output carries).
    pub offset: usize,
    /// Line ceiling for this call; `None` = the configured cap.
    pub limit: Option<usize>,
}

/// Parse read arguments.
fn parse_read_args(raw: &str) -> Result<ReadArgs> {
    let v: serde_json::Value = serde_json::from_str(raw)?;
    let path = v
        .get("path")
        .and_then(|c| c.as_str())
        .ok_or_else(|| anyhow!("missing `path` argument"))?
        .to_string();
    let offset = match v.get("offset").and_then(|o| o.as_u64()) {
        None => 1,
        Some(0) => return Err(anyhow!("offset 从 1 开始，收到 0")),
        Some(n) => n as usize,
    };
    let limit = v.get("limit").and_then(|l| l.as_u64()).map(|l| l as usize);
    if let Some(0) = limit {
        return Err(anyhow!("limit 至少是 1"));
    }
    Ok(ReadArgs {
        path,
        offset,
        limit,
    })
}

// read: show a file's contents (with line numbers, so the model can talk
// about specific lines and feed `mass_edit` later).
//
// Deliberately no output card in the UI: `edit`'s counterpart. The command
// (the path) is the whole story, and reading is side-effect-free — the
// transcript shows the upper card so the user sees what was read.
/// Read a file (with line numbers, so the model can talk about specific lines
/// and feed `mass_edit` later).
///
/// **Bounded on purpose.** A file bigger than the inline-output budget is read
/// only up to it: the model gets the head plus a notice naming the `offset` that
/// continues from there, and never a 300 MB file folded into the conversation.
/// The reader stops at the bound rather than reading-then-truncating, so the
/// memory cost is the bound, not the file.
pub fn read(
    cwd: &std::path::Path,
    zones: &[std::path::PathBuf],
    limits: crate::server::ai::config::OutputLimits,
    args: &ReadArgs,
) -> Result<ToolOutput> {
    use std::io::BufRead as _;
    let p = resolve_path(cwd, zones, &args.path, true)?;
    let max_lines = args.limit.unwrap_or(limits.max_lines).max(1);

    let file = std::fs::File::open(&p).context("failed to open file")?;
    let mut rdr = std::io::BufReader::new(file);
    let mut buf: Vec<u8> = Vec::new();
    let mut out = String::new();
    let mut line_no = 0usize;
    let mut shown = 0usize;
    let mut bytes = 0usize;
    let mut more = false;
    let mut by_bytes = false;
    loop {
        buf.clear();
        if rdr.read_until(b'\n', &mut buf)? == 0 {
            break; // EOF
        }
        line_no += 1;
        if line_no < args.offset {
            continue;
        }
        let line = std::str::from_utf8(&buf)
            .map_err(|_| anyhow!("不是 UTF-8 文本文件（第 {line_no} 行无法解码）"))?;
        let line = line.trim_end_matches(['\n', '\r']);
        if shown >= max_lines {
            more = true;
            break;
        }
        if shown > 0 && bytes + line.len() > limits.max_bytes {
            more = true;
            by_bytes = true;
            break;
        }
        out.push_str(&format!("{:>5}\t{line}\n", line_no));
        bytes += line.len();
        shown += 1;
    }

    if shown == 0 {
        out = if args.offset > 1 {
            format!("（第 {} 行起没有内容）", args.offset)
        } else {
            "(empty file)".to_string()
        };
    } else if more {
        out.push_str(&format!(
            "\n[还有更多行；继续读用 offset={}]",
            args.offset + shown
        ));
        if by_bytes {
            out.push_str(&format!(
                "（已到 {} 字节上限，未读到文件尾，总行数未知）",
                limits.max_bytes
            ));
        }
    }

    let details = serde_json::json!({
        "kind": "file",
        "path": p.display().to_string(),
        "lines": shown,
        "bytes": bytes,
        "offset": args.offset,
        "more": more,
    });
    Ok(ToolOutput::with_details(out, details))
}

// Parse bash arguments.
fn parse_bash_args(raw: &str) -> Result<String> {
    let v: serde_json::Value = serde_json::from_str(raw)?;
    Ok(v.get("command")
        .and_then(|c| c.as_str())
        .ok_or_else(|| anyhow!("missing `command` argument"))?
        .to_string())
}

// bash: real shell, guarded by the workspace-license classifier.
// Pipes/redirects/combinators run free; the guard blocks out-of-zone
// deletes and classic disasters (see agent/bash_guard.rs).

// Gatekeeper and executor for the bash tool.
//
// `artifacts` (when attached) powers the #N virtual-file syntax: every
// standalone `#N` token is rewritten to a temp file holding artifact N's
// content, and the command may itself produce a new artifact if its
// output overflows the spill threshold.
/// Gate + executor for the bash tool, with an optional externally-armed
/// interrupt flag (the session's stop; Esc kills a running command instead
/// of waiting out its deadline). `None` = deadline-only (unit tests).
fn bash_with_stop(
    cwd: &std::path::Path,
    command: &str,
    artifacts: Option<&crate::server::agent::artifacts::ArtifactStore>,
    timeout: std::time::Duration,
    interrupt: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    t_limits: crate::server::ai::config::OutputLimits,
    progress: &mut dyn FnMut(ToolProgress),
) -> Result<ToolOutput> {
    let cmd = command.trim();
    anyhow::ensure!(!cmd.is_empty(), "empty command");
    anyhow::ensure!(!cmd.contains('\n'), "one command at a time (no newlines)");

    // #N virtual files: rewrite before the guard (rewritten paths are
    // read-only temp files in a private dir; the guard reads them like
    // any other path).
    let (cmd, tmp_dir) = match artifacts {
        Some(a) => {
            let (c, dir) = crate::server::agent::artifacts::resolve_refs(cmd, a)?;
            (c, dir) // dir: Option<PathBuf> — None when no refs resolved
        }
        None => (cmd.to_string(), None),
    };
    debug_assert!(
        tmp_dir
            .as_deref()
            .map(|d| d != std::env::temp_dir())
            .unwrap_or(true),
        "temp dir must be a dedicated subdir, never /tmp itself"
    );
    let zone = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());
    let result = match crate::server::agent::bash_guard::classify(&cmd, &zone) {
        crate::server::agent::bash_guard::Verdict::Block(hits) => {
            let ids: Vec<&str> = hits.iter().map(|h| h.rule_id).collect();
            anyhow::bail!("denied: {}", ids.join("+"));
        }
        crate::server::agent::bash_guard::Verdict::Warn(hits) => {
            // High-tier: run, but tag the output so the model sees it.
            let tags: Vec<&str> = hits.iter().map(|h| h.rule_id).collect();
            let mut out = run_shell(cwd, &cmd, timeout, interrupt.clone(), progress)?;
            out.text = format!("[warn: {}]\n{}", tags.join("+"), out.text);
            out.warned = tags.iter().map(|t| t.to_string()).collect();
            Ok(out)
        }
        crate::server::agent::bash_guard::Verdict::Allow => {
            run_shell(cwd, &cmd, timeout, interrupt, progress)
        }
    };
    if let Some(dir) = tmp_dir {
        let _ = std::fs::remove_dir_all(dir); // temp materializations die with the command
    }
    let out = result?;
    // Spill: an overflowing output becomes an artifact; the context gets
    // the placeholder (which itself references #id for further use). The
    // artifact id travels in `details` too, so a front end can offer the whole
    // thing without parsing the placeholder sentence.
    let artifact = if let Some(a) = artifacts
        && crate::server::agent::artifacts::over_threshold(&out.text, t_limits)
    {
        let (id, total) = a.spill("bash", &out.text)?;
        let text = crate::server::agent::artifacts::placeholder(id, "bash", total, &out.text);
        Some((id, text))
    } else {
        None
    };
    let details = serde_json::json!({
        "kind": "shell",
        "exit_code": out.exit_code,
        "interrupted": out.interrupted,
        "warned": out.warned,
        "artifact": artifact.as_ref().map(|(id, _)| *id),
    });
    let text = match artifact {
        Some((_, text)) => text,
        None => out.text,
    };
    Ok(ToolOutput::with_details(text, details))
}

/// One finished shell command: the model-facing text plus the facts the UI
/// shows next to it. Kept apart from `ToolOutput` because the warn prefix and
/// the artifact spill both rewrite the text *after* the command is done.
struct ShellOutcome {
    text: String,
    exit_code: Option<i32>,
    interrupted: bool,
    warned: Vec<String>,
}

// bash -c execution with captured output. One command, no newlines —
// checked by the caller.
//
// Ownership of the child lives here, not with coreutils `timeout`:
//
// * `setsid --wait bash -c cmd` — util-linux's setsid makes bash the
//   **session and group leader** of a brand-new group, and `--wait` keeps
//   the setsid process in the foreground until bash exits. The direct child
//   (setsid) is what we spawn/reap; bash's pgid equals setsid's pid, so a
//   single `kill -SIG -pgid` reaches bash *and* every pipe/background
//   grandchild. Killing only bash would leave `b` of `a | b` holding the
//   stdout pipe open and the reader blocked forever — a group signal is
//   the entire point.
// * A waiter thread blocks on the child and forwards the output over a
//   channel; the tool thread `recv`s with a short timeout and checks the
//   stop flag between ticks. Stop (front-end Esc) and the wall-clock
//   deadline arm the same flag — one kill path for both: SIGTERM to the
//   group, SIGKILL two seconds later for anything that ignores it.
fn run_shell(
    cwd: &std::path::Path,
    cmd: &str,
    timeout: std::time::Duration,
    external: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    progress: &mut dyn FnMut(ToolProgress),
) -> Result<ShellOutcome> {
    let stop = std::sync::Arc::new(StopFlag::default());
    if let Some(ext) = external {
        // Bridge the session's AtomicBool into this command's flag: poll at
        // the same tick the wait loop uses. Interrupts are rare; 50ms of
        // worst-case latency is nothing next to a 600s deadline.
        let bridged = stop.clone();
        std::thread::spawn(move || {
            while !bridged.is_armed() {
                if ext.load(std::sync::atomic::Ordering::Relaxed) {
                    bridged.arm();
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(50));
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

    // Everything that happens to this command travels on one channel: the two
    // output streams and the exit status. Line-oriented on purpose — a chunk
    // boundary is not a character boundary, and it is not a line boundary
    // either, so the readers reassemble whole lines before saying anything.
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
        std::thread::spawn(move || {
            use std::io::Read as _;
            let mut rdr = std::io::BufReader::new(pipe);
            let mut raw = [0u8; 8192];
            let mut pending = String::new();
            loop {
                match rdr.read(&mut raw) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        // Lossy per read, then keep the trailing partial line
                        // back: a multi-byte character or an escape sequence
                        // split across reads must not be emitted half-formed.
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
    std::thread::spawn(move || {
        let r = child.wait();
        let _ = tx_wait.send(Msg::Exited(r));
        drop(waiter_stop);
    });
    drop(tx);

    // Deadline: arms the same flag the front-end stop arms.
    let timer_stop = stop.clone();
    std::thread::spawn(move || {
        std::thread::sleep(timeout);
        timer_stop.arm();
    });

    // Tool thread: drain with ticks; on stop, escalate TERM then KILL. Output
    // is forwarded to the front end as it arrives (throttled), so a command
    // that prints for a minute is visibly alive instead of a frozen card.
    const TICK: std::time::Duration = std::time::Duration::from_millis(50);
    /// How often the live view is refreshed. Fast enough to look live, slow
    /// enough that a chatty command cannot drown the front end.
    const REPORT_EVERY: std::time::Duration = std::time::Duration::from_millis(120);

    let mut stdout_text = String::new();
    let mut stderr_text = String::new();
    let mut status: Option<std::io::Result<std::process::ExitStatus>> = None;
    let mut readers_left = 2usize;
    let mut unflushed = String::new();
    let mut last_report = std::time::Instant::now();
    let mut killed = false;

    while readers_left > 0 || status.is_none() {
        match rx.recv_timeout(TICK) {
            Ok(Msg::Out(chunk)) => {
                stdout_text.push_str(&chunk);
                unflushed.push_str(&chunk);
            }
            Ok(Msg::Err(chunk)) => {
                stderr_text.push_str(&chunk);
                unflushed.push_str(&chunk);
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
    let killed = killed || stop.is_armed();
    let out = status
        .expect("loop only exits with a status")
        .with_context(|| "waiting for bash failed")?;

    // Strip the command's own ANSI escapes before the text goes anywhere.
    let stdout = crate::ansi::strip_ansi(&stdout_text);
    let err = crate::ansi::strip_ansi(&stderr_text);
    let mut text = stdout;
    if !err.trim().is_empty() {
        text.push_str("\n[stderr] ");
        text.push_str(err.trim_end());
    }
    if !out.success() {
        let code = out.code().unwrap_or(-1);
        if killed {
            text.push_str(&format!(
                "\n[interrupted/timeout: killed after {}s]",
                timeout.as_secs().max(1)
            ));
        } else {
            text.push_str(&format!("\n[exit {code}]"));
        }
    }
    Ok(ShellOutcome {
        text: text.trim_end().to_string(),
        exit_code: out.code(),
        interrupted: killed,
        warned: Vec::new(),
    })
}

/// Why a bash command died without a clean exit. Shared with the waiter
/// and deadline threads; arming is one-way.
#[derive(Default)]
struct StopFlag {
    armed: std::sync::atomic::AtomicBool,
}

impl StopFlag {
    fn arm(&self) {
        self.armed
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }
    fn is_armed(&self) -> bool {
        self.armed.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// Signal a whole process group via coreutils `kill` (negative pid = group).
/// `kill` is in util-linux/coreutils on every target box; when it is not,
/// the SIGKILL fallback on the direct child still lands via the waiter.
fn group_signal(pgid: u32, sig: &str) {
    let _ = std::process::Command::new("kill")
        .args([format!("-{sig}"), format!("-{pgid}")])
        .status();
}

// Parse cd arguments.
fn parse_cd_args(raw: &str) -> Result<String> {
    let v: serde_json::Value = serde_json::from_str(raw)?;
    Ok(v.get("path")
        .and_then(|c| c.as_str())
        .ok_or_else(|| anyhow!("missing `path` argument"))?
        .to_string())
}

impl BuiltinTools {
    // cd tool: temporary migration. Writes back to the shared slot (when
    // attached) and returns a result description.
    fn tool_cd(&mut self, path: &str) -> Result<ToolOutput> {
        let p = path.trim();
        anyhow::ensure!(!p.is_empty(), "empty path");
        let target = if let Some(rest) = p.strip_prefix('~') {
            std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest)
        } else {
            std::path::PathBuf::from(p)
        };
        let target = if target.is_absolute() {
            target
        } else {
            self.cwd.join(target)
        };
        let real = target
            .canonicalize()
            .with_context(|| format!("no such dir: {p}"))?;
        anyhow::ensure!(real.is_dir(), "not a directory: {p}");
        // The workspace is the boundary for every file tool, so `cd` — the only
        // way the model moves it — has to respect it. Out-of-workspace work is
        // still reachable by absolute path (the temp dir is licensed for files),
        // but the *session's* directory stays where the session started.
        let root = canonicalize_lenient(&self.root);
        anyhow::ensure!(
            real.starts_with(&root),
            "denied: {} is outside the workspace ({})",
            real.display(),
            root.display()
        );
        self.cwd = real.clone();
        // Write back to the shared slot: the TUI statusline and the next
        // turn's spawn_turn snapshot both read it
        if let Some(slot) = &self.cwd_slot {
            *slot.write().expect("cwd 锁中毒") = real.clone();
        }
        Ok(ToolOutput::with_details(
            format!("working directory changed to {}", real.display()),
            serde_json::json!({ "kind": "cwd", "path": real.display().to_string() }),
        ))
    }
}

// Parse edit arguments. A free function so unit tests can exercise it
// without a model.
pub fn parse_edit_args(arguments: &str) -> Result<EditArgs> {
    let v: serde_json::Value =
        serde_json::from_str(arguments).context("arguments is not valid JSON")?;
    Ok(EditArgs {
        path: need_str(&v, "path")?,
        old: need_str(&v, "old")?,
        new: need_str(&v, "new")?,
    })
}

// Run edit: find `old` in the file — it must occur exactly once — and
// replace it with `new`.
pub fn edit(
    cwd: &std::path::Path,
    zones: &[std::path::PathBuf],
    args: &EditArgs,
) -> Result<ToolOutput> {
    let p = resolve_path(cwd, zones, &args.path, true)?;
    let text = std::fs::read_to_string(&p).context("failed to read file")?;

    let count = text.matches(&args.old).count();
    match count {
        0 => Err(anyhow!(
            "old text not found; it must match the file content exactly (including whitespace and newlines)"
        )),
        1 => {
            // Where the replaced block starts — computed **before** the write,
            // while the old text is still in place.
            let at = text.find(&args.old).map(|i| line_of(&text, i));
            let updated = text.replacen(&args.old, &args.new, 1);
            std::fs::write(&p, updated).context("failed to write file")?;
            // The diff rides in `details`: `edit` knows exactly what it replaced,
            // so the UI never has to guess it back out of the result sentence.
            Ok(ToolOutput::with_details(
                format!("replaced: {}", p.display()),
                diff_details(&args.old, &args.new, at),
            ))
        }
        n => Err(anyhow!(
            "old appears {n} times; edit requires a unique match. \
             Use mass_edit to replace by line number, or make old \
             longer with surrounding context so it is unique"
        )),
    }
}

// ---------------------------------------------------------------------------
// write
// ---------------------------------------------------------------------------

/// Arguments for `write`: a whole file, and the whole content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriteArgs {
    pub path: String,
    pub content: String,
}

pub fn parse_write_args(arguments: &str) -> Result<WriteArgs> {
    let v: serde_json::Value =
        serde_json::from_str(arguments).context("arguments is not valid JSON")?;
    Ok(WriteArgs {
        path: need_str(&v, "path")?,
        content: need_str(&v, "content")?,
    })
}

/// Write a whole file, **overwriting** an existing one.
///
/// The counterpart to `edit`: `edit` refuses an ambiguous match, `write` refuses
/// nothing — it replaces the file outright, which is what "create this file"
/// and "this file is wrong from top to bottom" both need. The caller is
/// responsible for having the whole content; that is why the tool takes
/// `content` and not a patch.
pub fn write(
    cwd: &std::path::Path,
    zones: &[std::path::PathBuf],
    args: &WriteArgs,
) -> Result<ToolOutput> {
    let p = resolve_path(cwd, zones, &args.path, false)?;
    if let Some(parent) = p.parent() {
        anyhow::ensure!(
            parent.is_dir(),
            "目录不存在：{}（先用 bash mkdir -p 建目录）",
            parent.display()
        );
    }
    let existed = p.exists();
    std::fs::write(&p, &args.content).context("failed to write file")?;
    let bytes = args.content.len();
    let details = serde_json::json!({
        "kind": "write",
        "path": p.display().to_string(),
        "bytes": bytes,
        "created": !existed,
    });
    Ok(ToolOutput::with_details(
        format!(
            "{} {}（{bytes} 字节）",
            if existed { "已覆盖" } else { "已新建" },
            p.display()
        ),
        details,
    ))
}

// ---------------------------------------------------------------------------
// mass_edit
// ---------------------------------------------------------------------------

// One mass_edit instruction: replace line `line` entirely with `text`.
#[derive(Debug, Clone, PartialEq)]
pub struct LineEdit {
    pub line: usize,
    pub text: String,
}

#[derive(Debug)]
pub struct MassEditArgs {
    pub path: String,
    pub edits: Vec<LineEdit>,
}

// Parse mass_edit arguments.
//
// Two shapes are accepted (models emit both; being lenient saves a
// round-trip):
//
// ```json
// {"path":"x.txt","edits":[{"line":1,"text":"new content"}]}
// {"path":"x.txt","lines":[1,2],"text":"same text for all"}
// ```
pub fn parse_mass_edit_args(arguments: &str) -> Result<MassEditArgs> {
    let v: serde_json::Value =
        serde_json::from_str(arguments).context("arguments 不是合法 JSON")?;
    let path = need_str(&v, "path")?;

    if let Some(list) = v.get("edits").and_then(|x| x.as_array()) {
        let mut edits = Vec::with_capacity(list.len());
        for e in list {
            edits.push(LineEdit {
                line: need_line(e, "line")?,
                text: need_str(e, "text")?,
            });
        }
        return Ok(MassEditArgs { path, edits });
    }

    // Shorthand shape: the same text for multiple lines
    if let (Some(lines), Some(text)) = (
        v.get("lines").and_then(|x| x.as_array()),
        v.get("text").and_then(|x| x.as_str()),
    ) {
        let edits = lines
            .iter()
            .map(|l| {
                let line = l
                    .as_u64()
                    .ok_or_else(|| anyhow!("non-numeric entry in `lines`"))?
                    as usize;
                if line == 0 {
                    return Err(anyhow!("行号从 1 开始，收到 0"));
                }
                Ok(LineEdit {
                    line,
                    text: text.to_string(),
                })
            })
            .collect::<Result<Vec<_>>>()?;
        return Ok(MassEditArgs { path, edits });
    }

    Err(anyhow!(
        "mass_edit requires `edits: [{{line, text}}]` or `lines: [..]` + `text`"
    ))
}

// Run mass_edit: replace whole lines by number.
//
// Line numbers are 1-based; edits are applied **largest line first** so
// that editing line 10 does not invalidate line 2 — line numbers are
// stable indexes into the file, and walking from the tail prevents them
// from stepping on each other.
pub fn mass_edit(
    cwd: &std::path::Path,
    zones: &[std::path::PathBuf],
    args: &MassEditArgs,
) -> Result<ToolOutput> {
    let p = resolve_path(cwd, zones, &args.path, true)?;
    let text = std::fs::read_to_string(&p).context("failed to read file")?;

    let total = text.lines().count();
    let mut lines: Vec<String> = text.lines().map(|s| s.to_string()).collect();
    // Without a trailing newline, lines() yields no empty tail and the
    // write-back does not add one; preserved as-is

    let mut edits = args.edits.clone();
    edits.sort_by_key(|e| std::cmp::Reverse(e.line)); // largest first

    // Two edits for the same line = contradictory arguments; refusing is
    // safer than silently honoring the last one
    for w in edits.windows(2) {
        if w[0].line == w[1].line {
            return Err(anyhow!("line {} was specified twice", w[0].line));
        }
    }
    if let Some(e) = edits.last().filter(|e| e.line > total) {
        return Err(anyhow!(
            "line {} does not exist; file has {total} lines",
            e.line
        ));
    }

    let mut changed = 0usize;
    let mut before = Vec::new();
    let mut after = Vec::new();
    let at = edits.first().map(|e| e.line);
    for e in edits {
        // Captured before the overwrite: the diff is the tool's own knowledge,
        // not something the renderer should reconstruct from the sentence below.
        before.push(lines[e.line - 1].clone());
        after.push(e.text.clone());
        lines[e.line - 1] = e.text;
        changed += 1;
    }

    let mut out = lines.join("\n");
    if text.ends_with('\n') {
        out.push('\n');
    }
    std::fs::write(&p, out).context("failed to write file")?;
    Ok(ToolOutput::with_details(
        format!("replaced {changed} line(s) in {}", p.display()),
        diff_details(&before.join("\n"), &after.join("\n"), at),
    ))
}

/// The `details` payload for a text replacement: the two sides, line-split,
/// plus **where in the file** the block starts.
///
/// One shape for both editors — the UI draws a diff without knowing which tool
/// produced it, and `kind` is what it dispatches on. `at_line` is what lets the
/// renderer draw a line-number gutter (omp's `-315│…`); `None` when the tool
/// genuinely cannot say, and the gutter then just omits the number.
fn diff_details(old: &str, new: &str, at_line: Option<usize>) -> serde_json::Value {
    serde_json::json!({
        "kind": "diff",
        "deletions": old.lines().collect::<Vec<_>>(),
        "insertions": new.lines().collect::<Vec<_>>(),
        "atLine": at_line,
    })
}

/// 1-based line number of byte offset `pos` in `text` (clamped to the last line).
fn line_of(text: &str, pos: usize) -> usize {
    text[..pos.min(text.len())].matches('\n').count() + 1
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

// The real tool executor: dispatches calls the model places.
//
// Holds the session cwd — tool-relative paths resolve against the
// session directory, not whatever directory some thread happens to be in.
pub struct BuiltinTools {
    cwd: std::path::PathBuf,
    // The workspace **root**: where the session started, and the one thing `cd`
    // may not leave. `cwd` moves; the root does not, so "inside the workspace"
    // stays a stable fact even after a migration (a boundary that moves with the
    // thing it bounds is not a boundary).
    root: std::path::PathBuf,
    // Inline-output budget (config.yaml → tools.outputMax*): what `read` hands
    // over before pointing at the rest, and what makes a result a 巨物.
    limits: crate::server::ai::config::OutputLimits,
    // The cd tool writes its new directory here (read by the TUI
    // statusline and the next turn's tools).
    // None = not attached (unit tests); migration then affects only this turn.
    cwd_slot: Option<std::sync::Arc<std::sync::RwLock<std::path::PathBuf>>>,
    // Oversized tool outputs spill here. None = artifact mode off (unit
    // tests, or no DB): results pass through untruncated as before.
    artifacts: Option<crate::server::agent::artifacts::ArtifactStore>,
    // The model's todo list: `current` is what the `todo` tool reads and
    // rewrites, `pending` is the state nobody has published yet (the turn
    // runner drains it into a `SessionEvent::Todo` when the round ends).
    //
    // A shared slot rather than tool-internal state for the same reason `cd`
    // uses one: the tool layer must not know about the session, and the session
    // must not know about tools.
    todo: std::sync::Arc<std::sync::Mutex<crate::server::agent::todo::TodoState>>,
    // Finalized pre-turn history + cwd migrations: the `context` tool's
    // read-only view of "how did we get here". Snapshot semantics — the
    // current turn's own calls are NOT inside (the model just saw them).
    history: std::sync::Arc<Vec<crate::server::entry::Entry>>,
    cwd_trail: std::sync::Arc<Vec<(i64, String)>>,
    // Profile gate: `None` = all tools; `Some(names)` = only those are
    // registered into the Context *and* accepted by execute(). Two gates
    // on purpose: the roster keeps the model from ever seeing a disabled
    // tool, the executor refuses hallucinated calls into the void.
    enabled: Option<std::collections::BTreeSet<String>>,
    // Wall-clock cap on one `bash` command (config: `tools.bashTimeoutSecs`,
    // default 600). A hung command (`sleep 9999`, waiting on stdin, a
    // wedged network read) otherwise blocks the turn thread forever — the
    // interrupt flag is only polled from the stream callback, which never
    // runs while bash is executing.
    bash_timeout: std::time::Duration,
    // Session-level stop, armed by the front end's Esc/interrupt message.
    // When a bash command runs, the executor derives its kill flag from this
    // so a stop takes the command's whole process group down immediately.
    // `None` (unit tests, no session) = deadline-only.
    interrupt: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    // Which browser the web tools drive (config.yaml → `browser:`). Inert
    // without the `web` feature.
    browser: crate::server::ai::config::BrowserConfig,
}

impl BuiltinTools {
    pub fn new(cwd: std::path::PathBuf) -> Self {
        let root = cwd.clone();
        Self {
            cwd,
            root,
            limits: crate::server::ai::config::OutputLimits::default(),
            cwd_slot: None,
            artifacts: None,
            todo: std::sync::Arc::new(std::sync::Mutex::new(Default::default())),
            history: std::sync::Arc::new(Vec::new()),
            cwd_trail: std::sync::Arc::new(Vec::new()),
            enabled: None,
            // The default lives with the config schema (`ai::config`), so a
            // bare `BuiltinTools` (unit tests) and a configured one agree.
            bash_timeout: std::time::Duration::from_secs(
                crate::server::ai::config::ToolsConfig::default().bash_timeout_secs,
            ),
            interrupt: None,
            browser: Default::default(),
        }
    }

    /// Attach the session's interrupt flag (Arc<AtomicBool>, shared with the
    /// turn request).
    pub fn with_interrupt(
        mut self,
        flag: std::sync::Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        self.interrupt = Some(flag);
        self
    }

    // Attach the browser settings for the web tools (config.yaml → `browser:`).
    pub fn with_browser(mut self, cfg: crate::server::ai::config::BrowserConfig) -> Self {
        self.browser = cfg;
        self
    }

    /// Hand the tool layer the list as it stands (the turn runner reads the
    /// session's last `Entry::Todo` and seeds this before the round).
    pub fn with_todo(self, phases: Vec<crate::server::entry::TodoPhase>) -> Self {
        if let Ok(mut st) = self.todo.lock() {
            st.current = phases;
        }
        self
    }

    /// Take the list the tools produced this round (`None` = nothing changed).
    pub fn take_todo(&self) -> Option<Vec<crate::server::entry::TodoPhase>> {
        self.todo.lock().ok().and_then(|mut st| st.pending.take())
    }

    /// The workspace root: `cd` may move within it, never out of it.
    pub fn with_workspace_root(mut self, root: std::path::PathBuf) -> Self {
        self.root = root;
        self
    }

    /// The inline-output budget (config: `tools.outputMaxLines`/`outputMaxBytes`).
    pub fn with_limits(mut self, limits: crate::server::ai::config::OutputLimits) -> Self {
        self.limits = limits;
        self
    }

    /// The zones the file tools may touch (workspace + temp).
    fn zones(&self) -> Vec<std::path::PathBuf> {
        licensed_zones(&self.root)
    }

    // Attach the bash wall-clock cap (config: `tools.bashTimeoutSecs`).
    pub fn with_bash_timeout(mut self, secs: u64) -> Self {
        self.bash_timeout = std::time::Duration::from_secs(secs);
        self
    }

    // Profile gate: restrict which tools exist (definitions + execute).
    // `None` re-enables everything.
    pub fn with_enabled(mut self, names: Option<Vec<String>>) -> Self {
        self.enabled = names.map(|v| v.into_iter().collect());
        self
    }

    // Is `name` allowed through the profile gate?
    fn allowed(&self, name: &str) -> bool {
        match &self.enabled {
            None => true,
            Some(set) => set.contains(name),
        }
    }

    /// **The enablement interface.** Is this tool available to the model right
    /// now? True means both compiled in and allowed by the active profile —
    /// the two gates `execute` checks, answered in one place so a caller (a
    /// status panel, `mypi tools`, a test) never has to re-derive them.
    pub fn is_enabled(&self, name: &str) -> bool {
        match TOOLS.iter().find(|t| t.name == name) {
            Some(spec) => (spec.available)() && (!spec.filterable || self.allowed(spec.name)),
            None => false,
        }
    }

    /// Every tool name this session may use, in roster order.
    pub fn enabled_tools(&self) -> Vec<&'static str> {
        TOOLS
            .iter()
            .filter(|t| (t.available)() && (!t.filterable || self.allowed(t.name)))
            .map(|t| t.name)
            .collect()
    }
    // Attach the shared cwd slot: cd migrations become visible to the
    // TUI and subsequent turns immediately.
    pub fn with_cwd_slot(
        mut self,
        slot: std::sync::Arc<std::sync::RwLock<std::path::PathBuf>>,
    ) -> Self {
        self.cwd_slot = Some(slot);
        self
    }

    // Attach the finalized history + cwd trail for the `context` tool.
    pub fn with_history(
        mut self,
        history: std::sync::Arc<Vec<crate::server::entry::Entry>>,
        cwd_trail: std::sync::Arc<Vec<(i64, String)>>,
    ) -> Self {
        self.history = history;
        self.cwd_trail = cwd_trail;
        self
    }

    // Attach the artifact spill store (session-scoped, DB-backed).
    // `None` detaches: oversized output then flows into the context
    // verbatim (store-unavailable degradation).
    pub fn with_artifacts(mut self, store: crate::server::agent::artifacts::ArtifactStore) -> Self {
        self.artifacts = Some(store);
        self
    }

    pub fn with_artifacts_opt(
        mut self,
        store: Option<crate::server::agent::artifacts::ArtifactStore>,
    ) -> Self {
        self.artifacts = store;
        self
    }

    // The roster the model sees: the table, filtered by availability and the
    // profile's allow-list. One rule, applied once — the executor's gate reads
    // the same fields, so a tool can never be advertised and then refused (or
    // hidden and then accepted) through a second, drifted code path.
    pub fn definitions_for(&self) -> Vec<crate::server::ai::types::ToolDef> {
        TOOLS
            .iter()
            .filter(|t| (t.available)() && (!t.filterable || self.allowed(t.name)))
            .map(spec_to_def)
            .collect()
    }

    // Every compiled-in tool, ignoring the profile. Used by `mypi tools` and
    // by tests; the model-facing roster is `definitions_for`.
    pub fn definitions() -> Vec<crate::server::ai::types::ToolDef> {
        TOOLS
            .iter()
            .filter(|t| (t.available)())
            .map(spec_to_def)
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Registry — one table, everything derived
// ---------------------------------------------------------------------------
//
// A tool used to be knowledge scattered over the codebase: its manual here, its
// dispatch there, its schema's `intent` field copy-pasted nine times, its
// availability spelled out twice (roster + a `not(feature)` arm in the
// executor), its profile exception hardcoded in two places. Adding one tool
// meant finding all of them.
//
// Now there is one row per tool. `definitions_for`, `definitions` and
// `execute` are all *derived* from `TOOLS`, so the compiler checks the table
// once and the three readers cannot disagree.

/// How a tool wants its `intent` argument handled.
///
/// `intent` is the model's one-line "what am I about to do", shown to the user
/// while the tool blocks. It is injected into the schema (never hand-written
/// per tool) so the wording stays identical everywhere.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentPolicy {
    /// Injected as a required field. The default: a tool call the user cannot
    /// understand is worse than one extra token.
    Require,
    /// Injected, but the model may omit it.
    Optional,
    /// Not injected — for tools whose purpose is self-evident from their
    /// arguments (`cd` to a path needs no sentence).
    Omit,
}

/// The capability tier a tool exercises: what a reader checks to answer "can
/// this tool change my disk?" without reading the implementation.
///
/// Declared, not inferred: an approval gate, a sandbox policy or a "dry run"
/// mode reads this field, and a tool that lies about its tier is a bug the
/// table makes visible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tier {
    /// Reads state, changes nothing outside the process.
    Read,
    /// Writes files.
    Write,
    /// Runs commands / drives an external process.
    Exec,
}

/// A custom event a tool may emit, beyond the two every call gets.
///
/// `start` and `end` are **not** declared per tool: the loop emits them around
/// every call, so no tool can forget them and no front end has to check whether
/// a tool bothered. This table lists only the extras (progress, partial output,
/// ...), which are the tool's own business.
pub struct ToolEventDecl {
    /// Stable name, as it appears on the wire.
    pub name: &'static str,
    /// What it carries, for whoever wires it up next.
    pub description: &'static str,
}

/// How a tool is invoked: the raw argument string, the progress sink, the result.
///
/// Named because it appears on every row of [`TOOLS`] and as the signature of
/// ten adapter functions — spelled out inline it reads like noise.
pub type ToolRun =
    fn(&mut BuiltinTools, &str, &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput>;

/// Everything one tool is.
pub struct ToolSpec {
    /// The name the model calls, and the key the profile's allow-list uses.
    pub name: &'static str,
    /// The manual the model reads.
    pub description: &'static str,
    /// Tool-specific JSON-Schema object; `intent` is injected per [`IntentPolicy`].
    pub schema: fn() -> serde_json::Value,
    /// Parse the raw argument string and run. The only behavioural entry point.
    ///
    /// See [`ToolRun`] for the shape; the third parameter is the tool's channel
    /// to the user *while it runs* (a tool with nothing to report ignores it).
    pub run: ToolRun,
    /// Compiled in and usable right now (the web tools are feature-gated).
    pub available: fn() -> bool,
    /// Whether the profile's allow-list may hide this tool. `context` is the
    /// one that cannot be hidden: it is the compressed session's only way back
    /// into its own history.
    pub filterable: bool,
    pub intent: IntentPolicy,
    pub tier: Tier,
    /// Custom events beyond start/end. Empty = the loop's pair is all it emits.
    pub events: &'static [ToolEventDecl],
}

/// The model-facing manual of one tool: its schema with `intent` injected.
///
/// A panic here is a programming error, not user input: every schema in `TOOLS`
/// is a literal in this file, and the table's own test asserts the shape.
fn spec_to_def(spec: &ToolSpec) -> ToolDef {
    let mut schema = (spec.schema)();
    if spec.intent != IntentPolicy::Omit {
        let props = schema
            .get_mut("properties")
            .and_then(|p| p.as_object_mut())
            .expect("tool schema must have an object `properties`");
        props.insert(
            "intent".to_string(),
            serde_json::json!({
                "type": "string",
                "description": "一句话说明这次调用要干什么，中文，会显示给用户看"
            }),
        );
        if spec.intent == IntentPolicy::Require {
            schema
                .get_mut("required")
                .and_then(|r| r.as_array_mut())
                .expect("tool schema must have a `required` array")
                .push(serde_json::json!("intent"));
        }
    }
    ToolDef::function(spec.name, spec.description, schema)
}

/// A tool's JSON-Schema object with `properties` + `required` already in place.
fn schema_of(properties: serde_json::Value, required: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": required,
    })
}

// ---------------------------------------------------------------------------
// The table
// ---------------------------------------------------------------------------

/// Every built-in tool. Order is the order the model reads them in.
pub const TOOLS: &[ToolSpec] = &[
    ToolSpec {
        name: "read",
        description: "读文件（带行号，便于后面引用具体行）。改文件前先读，避免瞎猜原文。\
                      只读文件，不读目录；只读工作区（会话起始目录）与 /tmp 里的文件。\
                      文件超过本工具的上限时只读前面一段，末尾会提示用 offset 续读。",
        schema: || {
            schema_of(
                serde_json::json!({
                    "path": {"type": "string", "description": "文件路径，支持 ./ 与 ../"},
                    "offset": {"type": "integer", "description": "从第几行开始读（1 起，默认 1）。输出末尾会给出续读用的 offset"},
                    "limit": {"type": "integer", "description": "这次最多读几行，默认取配置上限"}
                }),
                &["path"],
            )
        },
        run: run_read,
        available: || true,
        filterable: true,
        intent: IntentPolicy::Require,
        tier: Tier::Read,
        events: &[],
    },
    ToolSpec {
        name: "edit",
        description: "在文件里把一段文本替换成另一段。old 必须在文件里恰好出现一次。\
                      适合改一个有唯一上下文的位置。只改工作区与 /tmp 里的文件。",
        schema: || {
            schema_of(
                serde_json::json!({
                    "path": {"type": "string", "description": "文件路径，支持 ./ 与 ../"},
                    "old": {"type": "string", "description": "要被替换的原文，必须逐字符匹配且唯一"},
                    "new": {"type": "string", "description": "替换成的内容"}
                }),
                &["path", "old", "new"],
            )
        },
        run: run_edit,
        available: || true,
        filterable: true,
        intent: IntentPolicy::Require,
        tier: Tier::Write,
        events: &[],
    },
    ToolSpec {
        name: "write",
        description: "写入一个文件的全部内容；同名文件已存在就**整个覆盖**。\
                      适合新建文件，或整份内容都要换掉的场合。只想改一处用 edit。\
                      只写工作区（会话起始目录）与 /tmp 里的文件；父目录不存在会拒绝。",
        schema: || {
            schema_of(
                serde_json::json!({
                    "path": {"type": "string", "description": "文件路径，支持 ./ 与 ../"},
                    "content": {"type": "string", "description": "文件的完整内容（会被原样写入）"}
                }),
                &["path", "content"],
            )
        },
        run: run_write,
        available: || true,
        filterable: true,
        intent: IntentPolicy::Require,
        tier: Tier::Write,
        events: &[],
    },
    ToolSpec {
        name: "cd",
        description: "临时切换工作目录（本会话内有效）。之后的工具调用与相对路径都以新目录为基准。\
                      只接受目录，且只能在当前工作区内迁移（工作区=会话起始目录）；\
                      用 pwd 或 ls 确认切换结果。",
        schema: || {
            schema_of(
                serde_json::json!({
                    "path": {"type": "string", "description": "目标目录，支持 ./ ../ ~/ 与绝对路径"}
                }),
                &["path"],
            )
        },
        run: run_cd,
        available: || true,
        filterable: true,
        intent: IntentPolicy::Require,
        tier: Tier::Read,
        events: &[],
    },
    ToolSpec {
        name: "bash",
        description: "执行 shell 命令（bash -c 语义，单条命令，不接受换行）。\
                      管道、重定向、组合可用。工作目录就是许可区，删除/移动其中的东西随便；\
                      但 rm/mv/find -delete 一旦触及工作区之外（包括 ~、/etc 等系统路径）会被直接拒绝。\
                      dd 写盘、fork bomb、反弹 shell、curl|sh 等灾难模式同样被拦截。",
        schema: || {
            schema_of(
                serde_json::json!({
                    "command": {"type": "string", "description": "单条 shell 命令"}
                }),
                &["command"],
            )
        },
        run: run_bash,
        available: || true,
        filterable: true,
        intent: IntentPolicy::Require,
        tier: Tier::Exec,
        events: &[],
    },
    ToolSpec {
        name: "mass_edit",
        description: "按行号把文件的指定行整行替换。不搜索不匹配，指哪改哪。\
                      适合改动多行、或原文有重复行导致 edit 无法唯一定位的场合。\
                      只改工作区与 /tmp 里的文件。",
        schema: || {
            schema_of(
                serde_json::json!({
                    "path": {"type": "string", "description": "文件路径"},
                    "edits": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "line": {"type": "integer", "description": "行号，从 1 开始"},
                                "text": {"type": "string", "description": "这一行换成的内容"}
                            },
                            "required": ["line", "text"]
                        }
                    }
                }),
                &["path", "edits"],
            )
        },
        run: run_mass_edit,
        available: || true,
        filterable: true,
        intent: IntentPolicy::Require,
        tier: Tier::Write,
        events: &[],
    },
    #[cfg(feature = "web")]
    ToolSpec {
        name: "fetch",
        description: "读取一个网页，返回干净的 Markdown 正文（自动去导航/广告/页脚）。\
                      适合读文档、文章、搜到的结果页。JS 重渲染的页面会自动走浏览器兜底。\
                      输出截断在 24K 字符；raw: true 时返回原始 HTML。",
        schema: || {
            schema_of(
                serde_json::json!({
                    "url": {"type": "string", "description": "要读取的 URL（http/https，裸域名自动补 https://）"},
                    "raw": {"type": "boolean", "description": "返回原始 HTML 而不是 markdown，默认 false"}
                }),
                &["url"],
            )
        },
        run: run_fetch,
        available: || true,
        filterable: true,
        intent: IntentPolicy::Require,
        tier: Tier::Read,
        events: &[],
    },
    #[cfg(feature = "web")]
    ToolSpec {
        name: "browser",
        description: "操控真实浏览器（Chromium 内核，如 Helium）。四个命令：\
                      open（打开 URL）、act（交互：navigate/click/fill/press/select/scroll/eval/net）、\
                      read（把当前页面读成 markdown）、screenshot（截图存文件）。\
                      适合 JS 重渲染的 SPA、需要登录的站、需要点击/滚动/抓网络请求的场合。\
                      net 操作可列出页面发出的网络请求（找评论区的数据包接口就用它）。",
        schema: || {
            schema_of(
                serde_json::json!({
                    "command": {"type": "string", "enum": ["open", "act", "read", "screenshot"], "description": "操作类型"},
                    "url": {"type": "string", "description": "open/act=navigate: 要打开的 URL"},
                    "op": {"type": "string", "enum": ["navigate", "click", "fill", "press", "select", "scroll", "eval", "net"], "description": "act 的具体操作"},
                    "selector": {"type": "string", "description": "CSS 选择器（click/fill/select/scroll）或按键名（press，如 Enter）"},
                    "value": {"description": "fill: 文本；select: 选项值；scroll: 像素；eval: JS 表达式"},
                    "filter": {"type": "string", "description": "act=net: 只保留 URL 含此子串的请求"},
                    "max": {"type": "integer", "description": "act=net: 最多返回几条请求，默认 20"},
                    "path": {"type": "string", "description": "screenshot: PNG 保存路径；read: markdown 保存路径"},
                    "full_page": {"type": "boolean", "description": "screenshot: 整页截图，默认只截视口"}
                }),
                &["command"],
            )
        },
        run: run_browser,
        available: || true,
        filterable: true,
        intent: IntentPolicy::Require,
        // `act eval` runs arbitrary JS in a browser holding the user's real
        // profile (logins, extensions): that is execution, not reading.
        tier: Tier::Exec,
        events: &[],
    },
    #[cfg(feature = "web")]
    ToolSpec {
        name: "search",
        description: "网页搜索，返回标题、链接与摘要。支持高级语法：\
                      site:github.com（限定域名）、\"精确短语\"、-排除词、filetype:、inurl:、intitle:、before:/after:（YYYY-MM-DD）。\
                      返回 5-10 条结果；查实时信息、文档定位、找源码仓库时用它。",
        schema: || {
            schema_of(
                serde_json::json!({
                    "query": {"type": "string", "description": "搜索词，支持 site: \"短语\" -排除 filetype: inurl: intitle: before:/after:"},
                    "limit": {"type": "integer", "description": "返回条数，默认 8，最大 20"}
                }),
                &["query"],
            )
        },
        run: run_search,
        available: || true,
        filterable: true,
        intent: IntentPolicy::Require,
        tier: Tier::Read,
        events: &[],
    },
    ToolSpec {
        name: "todo",
        description: "维护一份任务清单，当作**外部备忘录**用：长任务先 init 建清单，做到哪就 start/done 哪一项。\
                      状态存在会话里（不靠你记），每次结果都会把**完整清单**回给你，随时可以 view 重读。\
                      task 必须与写下时逐字一致。一次只能有一项 in_progress（start 会自动把上一项退回 pending）。\
                      block 要带 reason；整批有错就整批不改。",
        schema: || {
            schema_of(
                serde_json::json!({
                    "op": {
                        "type": "string",
                        "enum": ["init", "append", "start", "done", "drop", "block", "unblock", "rm", "view"],
                        "description": "操作：init 建/换整份清单；append 加项；start/done/drop 改一项状态；block/unblock 卡住与解开；rm 删一项；view 只读"
                    },
                    "list": {
                        "type": "array",
                        "description": "init：阶段清单 [{phase, items}]",
                        "items": {
                            "type": "object",
                            "properties": {
                                "phase": {"type": "string", "description": "阶段名"},
                                "items": {"type": "array", "items": {"type": "string"}, "description": "这个阶段的任务"}
                            },
                            "required": ["phase", "items"]
                        }
                    },
                    "items": {"type": "array", "items": {"type": "string"}, "description": "init（平铺）或 append：任务文本"},
                    "phase": {"type": "string", "description": "append 往哪个阶段加，缺省落默认阶段"},
                    "task": {"type": "string", "description": "要操作的那一项的**原文**（逐字照抄）"},
                    "reason": {"type": "string", "description": "block：卡住的原因"}
                }),
                &["op"],
            )
        },
        run: run_todo,
        available: || true,
        filterable: true,
        intent: IntentPolicy::Require,
        // 它不动盘、不跑命令：它维护的是会话自己的一份笔记。
        tier: Tier::Read,
        events: &[],
    },
    ToolSpec {
        name: "context",
        description: "查询会话历史的调用记录（只读，不执行）。主要用于压缩后自查：\
                      巨物 #N（如 #1）是怎么产生的——传 anchor=\"#N\"（或工具结果的 call_id），\
                      返回它前后的工具调用序列，含 cwd 变动轨迹。\
                      anchor 也可以是一个 call_id 本身；before/after 控制前后各看几条；\
                      include_user=true 时把用户消息也列出来（看需求变动）；\
                      include_results=true 时附上每次调用的结果全文（看报错）。",
        schema: || {
            schema_of(
                serde_json::json!({
                    "anchor": {"type": "string", "description": "锚点：巨物引用（\"#1\"）或某次工具结果的 call_id"},
                    "before": {"type": "integer", "description": "锚点前看几条工具调用，默认 10"},
                    "after": {"type": "integer", "description": "锚点后看几条工具调用，默认 10"},
                    "include_user": {"type": "boolean", "description": "是否包含用户消息，默认 false"},
                    "include_results": {"type": "boolean", "description": "是否包含工具结果全文，默认 false（只列调用）"}
                }),
                &["anchor"],
            )
        },
        run: run_context,
        available: || true,
        // Never hidden by a profile: it is the compressed session's escape
        // hatch back into its own history (the `context` tool is how a model
        // asks "where did artifact #1 come from").
        filterable: false,
        intent: IntentPolicy::Require,
        tier: Tier::Read,
        events: &[],
    },
];

// ---------------------------------------------------------------------------
// Adapters: `ToolSpec::run` → the tool's own function
// ---------------------------------------------------------------------------

fn run_read(
    t: &mut BuiltinTools,
    args: &str,
    _progress: &mut dyn FnMut(ToolProgress),
) -> Result<ToolOutput> {
    let zones = t.zones();
    read(&t.cwd, &zones, t.limits, &parse_read_args(args)?)
}

fn run_edit(
    t: &mut BuiltinTools,
    args: &str,
    _progress: &mut dyn FnMut(ToolProgress),
) -> Result<ToolOutput> {
    let zones = t.zones();
    edit(&t.cwd, &zones, &parse_edit_args(args)?)
}

fn run_write(
    t: &mut BuiltinTools,
    args: &str,
    _progress: &mut dyn FnMut(ToolProgress),
) -> Result<ToolOutput> {
    let zones = t.zones();
    write(&t.cwd, &zones, &parse_write_args(args)?)
}

fn run_mass_edit(
    t: &mut BuiltinTools,
    args: &str,
    _progress: &mut dyn FnMut(ToolProgress),
) -> Result<ToolOutput> {
    let zones = t.zones();
    mass_edit(&t.cwd, &zones, &parse_mass_edit_args(args)?)
}

fn run_cd(
    t: &mut BuiltinTools,
    args: &str,
    _progress: &mut dyn FnMut(ToolProgress),
) -> Result<ToolOutput> {
    t.tool_cd(&parse_cd_args(args)?)
}

fn run_bash(
    t: &mut BuiltinTools,
    args: &str,
    progress: &mut dyn FnMut(ToolProgress),
) -> Result<ToolOutput> {
    bash_with_stop(
        &t.cwd,
        &parse_bash_args(args)?,
        t.artifacts.as_ref(),
        t.bash_timeout,
        t.interrupt.clone(),
        t.limits,
        progress,
    )
}

fn run_todo(
    t: &mut BuiltinTools,
    args: &str,
    _progress: &mut dyn FnMut(ToolProgress),
) -> Result<ToolOutput> {
    use crate::server::agent::todo as td;
    let a = td::TodoArgs::parse(args)?;
    let mut st = t.todo.lock().map_err(|_| anyhow!("todo 锁中毒"))?;
    let (next, errors) = td::apply(&st.current, &a);
    if !errors.is_empty() {
        // 整批丢弃：状态一动不动，理由如实回报（模型据此改一次就能过）。
        return Err(anyhow!(
            "{} 没做成，清单未改动：\n- {}",
            a.op.as_str(),
            errors.join("\n- ")
        ));
    }
    if !a.op.is_read_only() {
        st.current = next.clone();
        st.pending = Some(next.clone());
    }
    Ok(ToolOutput::with_details(
        td::summary_text(&next),
        td::details(&next, a.op),
    ))
}

fn run_context(
    t: &mut BuiltinTools,
    args: &str,
    _progress: &mut dyn FnMut(ToolProgress),
) -> Result<ToolOutput> {
    context_query(&t.history, &t.cwd_trail, &parse_context_args(args)?)
}

// The web tools have two bodies each: the real one, and — in a build without
// the feature — one that refuses explicitly. Silence would be worse: the model
// would believe the call ran.
#[cfg(feature = "web")]
fn run_fetch(
    t: &mut BuiltinTools,
    args: &str,
    _progress: &mut dyn FnMut(ToolProgress),
) -> Result<ToolOutput> {
    let a = crate::web::parse_fetch_args(args)?;
    let text = crate::web::fetch(&a, &t.browser)?;
    Ok(ToolOutput::with_details(
        text,
        serde_json::json!({ "kind": "web", "url": a.url, "raw": a.raw.unwrap_or(false) }),
    ))
}
#[cfg(not(feature = "web"))]
fn run_fetch(
    _t: &mut BuiltinTools,
    _args: &str,
    _progress: &mut dyn FnMut(ToolProgress),
) -> Result<ToolOutput> {
    Err(anyhow!(
        "tool `fetch` is not compiled in (rebuild with --features web)"
    ))
}

#[cfg(feature = "web")]
fn run_browser(
    t: &mut BuiltinTools,
    args: &str,
    _progress: &mut dyn FnMut(ToolProgress),
) -> Result<ToolOutput> {
    let a = crate::web::parse_browser_args(args)?;
    let details = serde_json::json!({
        "kind": "browser",
        "command": a.command,
        "url": a.url,
        "op": a.op,
    });
    let text = crate::web::browser(&a, &t.browser)?;
    Ok(ToolOutput::with_details(text, details))
}
#[cfg(not(feature = "web"))]
fn run_browser(
    _t: &mut BuiltinTools,
    _args: &str,
    _progress: &mut dyn FnMut(ToolProgress),
) -> Result<ToolOutput> {
    Err(anyhow!(
        "tool `browser` is not compiled in (rebuild with --features web)"
    ))
}

#[cfg(feature = "web")]
fn run_search(
    t: &mut BuiltinTools,
    args: &str,
    _progress: &mut dyn FnMut(ToolProgress),
) -> Result<ToolOutput> {
    let a = crate::web::parse_search_args(args)?;
    let hits = crate::web::search(&a, &t.browser)?;
    let details = serde_json::json!({
        "kind": "search",
        "query": a.query,
        "hits": hits.len(),
    });
    Ok(ToolOutput::with_details(crate::web::render(&hits), details))
}
#[cfg(not(feature = "web"))]
fn run_search(
    _t: &mut BuiltinTools,
    _args: &str,
    _progress: &mut dyn FnMut(ToolProgress),
) -> Result<ToolOutput> {
    Err(anyhow!(
        "tool `search` is not compiled in (rebuild with --features web)"
    ))
}

// ---------------------------------------------------------------------------
// context tool — the history window query
// ---------------------------------------------------------------------------

// Arguments for the `context` tool.
#[derive(Debug, Clone)]
pub struct ContextArgs {
    pub anchor: String,
    pub before: usize,
    pub after: usize,
    pub include_user: bool,
    pub include_results: bool,
}

pub fn parse_context_args(arguments: &str) -> Result<ContextArgs> {
    let v: serde_json::Value =
        serde_json::from_str(arguments).context("arguments is not valid JSON")?;
    Ok(ContextArgs {
        anchor: need_str(&v, "anchor")?,
        before: v
            .get("before")
            .and_then(|b| b.as_u64())
            .map(|b| b as usize)
            .unwrap_or(10),
        after: v
            .get("after")
            .and_then(|a| a.as_u64())
            .map(|a| a as usize)
            .unwrap_or(10),
        include_user: v
            .get("include_user")
            .and_then(|u| u.as_bool())
            .unwrap_or(false),
        include_results: v
            .get("include_results")
            .and_then(|r| r.as_bool())
            .unwrap_or(false),
    })
}

// Scan the finalized history for the anchor (an artifact ref like "#1"
// inside a tool result, or a bare call_id) and render the surrounding
// tool-call window, annotated with cwd migrations. Read-only: it never
// executes anything and never touches the live turn's entries.
pub fn context_query(
    history: &[crate::server::entry::Entry],
    cwd_trail: &[(i64, String)],
    args: &ContextArgs,
) -> Result<ToolOutput> {
    use crate::server::entry::Entry;
    let anchor = args.anchor.trim();

    // Resolve the anchor to an index: a call_id matches a ToolRequest/
    // ToolResult; "#N" matches any tool whose *result* references the
    // artifact (the placeholder embeds `#N`).
    let anchor_idx = if let Some(num) = anchor.strip_prefix('#') {
        let id = num
            .trim()
            .parse::<u64>()
            .with_context(|| format!("巨物引用格式应为 #N：{anchor}"))?;
        let needle = format!("#{id}");
        history
            .iter()
            .position(|e| matches!(e, Entry::ToolResult { result, .. } if result.contains(&needle)))
            .with_context(|| format!("历史里没有巨物 {needle} 的产生记录"))?
    } else {
        history
            .iter()
            .position(|e| match e {
                Entry::ToolRequest { call_id, .. } | Entry::ToolResult { call_id, .. } => {
                    call_id == anchor
                }
                _ => false,
            })
            .with_context(|| format!("历史里没有 call_id {anchor}"))?
    };

    // Collect the (index, kind-of-row) pairs we will print: the anchor
    // entry itself ± before/after *tool* entries, optionally interleaving
    // user messages.
    let mut picks: Vec<usize> = Vec::new();
    for (i, e) in history.iter().enumerate() {
        let is_tool = matches!(e, Entry::ToolRequest { .. } | Entry::ToolResult { .. });
        let is_user = matches!(e, Entry::User { .. });
        let in_window = i >= anchor_idx.saturating_sub(args.before)
            && i <= anchor_idx.saturating_add(args.after);
        if in_window && (is_tool || (args.include_user && is_user)) {
            picks.push(i);
        }
    }
    let mut out = String::new();
    // cwd migration markers: (seq, path) — the trail's seq is the entry
    // index at which the migration was recorded. Emit "cwd changed to"
    // lines wherever a pick crosses one.
    let mut trail = cwd_trail.iter().peekable();
    for &i in &picks {
        while let Some((_seq, path)) = trail
            .peek()
            .map(|(s, p)| (*s, p.clone()))
            .filter(|(s, _)| *s <= i as i64)
        {
            out.push_str(&format!("cwd：{path}\n"));
            trail.next();
        }
        match &history[i] {
            Entry::User { content } => {
                out.push_str(&format!("user：{}\n", first_line(content)));
            }
            Entry::ToolRequest {
                call_id,
                name,
                args,
                ..
            } => {
                out.push_str(&format!(
                    "#{i} 调用 {name}（{call_id}）：{}\n",
                    first_line(args)
                ));
            }
            Entry::ToolResult {
                call_id,
                name,
                ok,
                result,
                ..
            } => {
                if args.include_results {
                    out.push_str(&format!(
                        "#{} 结果 {}（{}）：\n{}\n",
                        i,
                        name,
                        if *ok { "ok" } else { "FAILED" },
                        result
                    ));
                } else {
                    out.push_str(&format!(
                        "#{i} 结果 {name}（{}）{}\n",
                        call_id,
                        if *ok { "ok" } else { "FAILED" }
                    ));
                }
            }
            _ => {}
        }
    }
    if out.is_empty() {
        out.push_str("（窗口内没有可显示的调用记录——放宽 before/after 试试）");
    }
    Ok(ToolOutput::with_details(
        out,
        serde_json::json!({ "kind": "context", "anchor": args.anchor }),
    ))
}

fn first_line(s: &str) -> String {
    let line = s.lines().next().unwrap_or("");
    line.chars().take(120).collect()
}

impl super::loop_rs::ToolExecutor for BuiltinTools {
    fn execute(
        &mut self,
        call: &ToolCall,
        progress: &mut dyn FnMut(ToolProgress),
    ) -> Result<ToolOutput> {
        let spec = TOOLS
            .iter()
            .find(|t| t.name == call.name())
            .ok_or_else(|| anyhow!("unknown tool: {}", call.name()))?;
        // Gate one: compiled in. Gate two: allowed by the profile. Both read
        // the same table the roster was built from, so a call can never be
        // advertised and then refused (or hidden and then accepted) by a second
        // code path that drifted from the first.
        anyhow::ensure!(
            (spec.available)(),
            "tool `{}` is not compiled in (rebuild with --features web)",
            spec.name
        );
        anyhow::ensure!(
            !spec.filterable || self.allowed(spec.name),
            "tool `{}` is disabled by the active profile",
            spec.name
        );
        (spec.run)(self, &call.function.arguments, progress)
    }
}

#[cfg(test)]
mod tests {
    use super::super::loop_rs::ToolExecutor as _;
    use super::*;

    // Generous per-test timeout: the tests below only assert pass/deny
    // behaviour, never the cap itself (that has its own dedicated test).
    const T: std::time::Duration = std::time::Duration::from_secs(600);

    /// 测试里的默认输出上限（与 config 默认一致）。
    const L: crate::server::ai::config::OutputLimits = crate::server::ai::config::OutputLimits {
        max_lines: 512,
        max_bytes: 256 * 1024,
    };

    fn home_dir() -> std::path::PathBuf {
        std::env::var("HOME")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|_| std::env::temp_dir())
    }

    use serde_json::json;
    use std::path::Path;

    fn tempdir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("mypi-tools-{name}"));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[cfg(feature = "web")]
    #[test]
    fn search_tool_is_registered_with_schema() {
        let defs = BuiltinTools::definitions();
        let s = defs
            .iter()
            .find(|d| d.function.name == "search")
            .expect("search tool must be registered");
        // The model needs the operator list in the description — it is the
        // only docs for the syntax the engine honors.
        assert!(
            s.function.description.contains("site:"),
            "schema must document site: ({})",
            s.function.description
        );
        let props = &s.function.parameters["properties"];
        assert!(props.get("query").is_some());
        assert!(props.get("limit").is_some());
    }

    #[test]
    fn the_table_is_self_consistent() {
        // The table is now the single source for the roster, the executor's
        // gates and the schemas. These are the invariants the three readers
        // depend on, so they are asserted once here instead of trusted.
        let mut seen = std::collections::BTreeSet::new();
        for t in TOOLS {
            assert!(seen.insert(t.name), "工具名重复: {}", t.name);
            assert!(!t.description.trim().is_empty(), "{} 缺手册", t.name);
            let schema = (t.schema)();
            assert!(
                schema.get("properties").and_then(|p| p.as_object()).is_some(),
                "{} 的 schema 必须有 properties 对象",
                t.name
            );
            assert!(
                schema.get("required").and_then(|r| r.as_array()).is_some(),
                "{} 的 schema 必须有 required 数组",
                t.name
            );
            // `intent` is injected, never hand-written: a schema that already
            // carries it would silently get it twice.
            assert!(
                schema["properties"].get("intent").is_none(),
                "{} 的 schema 不该自己写 intent（由 IntentPolicy 注入）",
                t.name
            );
        }
        assert_eq!(seen.len(), TOOLS.len());
    }

    #[test]
    fn intent_is_injected_per_policy() {
        // Every built-in wants it, so every schema advertises it as required —
        // the point is that the field exists exactly once, injected.
        for def in BuiltinTools::definitions() {
            let props = &def.function.parameters["properties"];
            assert!(
                props.get("intent").is_some(),
                "{} 的 schema 缺注入的 intent",
                def.function.name
            );
            let required = def.function.parameters["required"]
                .as_array()
                .expect("required 数组");
            assert!(
                required.iter().any(|r| r == "intent"),
                "{} 的 intent 必须是 required",
                def.function.name
            );
        }
    }

    #[test]
    fn enablement_is_a_question_anyone_can_ask() {
        // The profile's allow-list is not a secret the executor keeps: the
        // enablement interface answers for both gates (compiled in + allowed),
        // so a status panel or `mypi tools` never re-derives them.
        let t = BuiltinTools::new(std::env::temp_dir());
        for name in ["read", "edit", "write", "bash", "context"] {
            assert!(t.is_enabled(name), "{name} 默认该是启用的");
        }
        assert!(!t.is_enabled("nope"), "不存在的工具不是启用的");

        let restricted =
            BuiltinTools::new(std::env::temp_dir()).with_enabled(Some(vec!["read".into()]));
        assert!(restricted.is_enabled("read"));
        assert!(!restricted.is_enabled("bash"));
        assert!(
            restricted.is_enabled("context"),
            "context 不可被 profile 关掉（压缩后的唯一退路）"
        );
        assert_eq!(restricted.enabled_tools(), vec!["read", "context"]);
    }

    #[test]
    fn the_todo_tool_reads_state_it_was_never_told() {
        // 这个工具的**全部意义**：模型只发 `{op:"done", task:"甲"}`，它凭这句
        // 就能改状态——因为清单不在模型脑子里，而在会话里（这里用槽模拟）。
        use crate::server::entry::{TodoPhase, TodoStatus, TodoTask};
        let seed = vec![TodoPhase {
            name: "阶段一".into(),
            tasks: vec![TodoTask {
                content: "甲".into(),
                status: TodoStatus::Pending,
                blocker: None,
            }],
        }];
        let mut t = BuiltinTools::new(std::env::temp_dir()).with_todo(seed);
        let call = crate::server::ai::types::ToolCall {
            id: "c".into(),
            kind: "function".into(),
            function: crate::server::ai::types::FunctionCall {
                name: "todo".into(),
                arguments: r#"{"op":"done","task":"甲"}"#.into(),
            },
        };
        let out = crate::server::agent::loop_rs::ToolExecutor::execute(&mut t, &call, &mut |_| {})
            .unwrap();
        assert!(out.text.contains("[x] 甲"), "{}", out.text);
        assert_eq!(
            out.details.unwrap()["done"],
            1,
            "结果要带全量状态给前端与模型"
        );
        // 改完的状态等着被发布（turn 取走后发 SessionEvent::Todo）。
        let published = t.take_todo().expect("改动必须能被取走");
        assert_eq!(published[0].tasks[0].status, TodoStatus::Done);
        assert!(t.take_todo().is_none(), "取过一次就不该再取到");
    }

    #[test]
    fn a_todo_batch_with_an_error_changes_nothing() {
        // 整批丢弃：不留半应用状态，模型重试不会撞"已经改过了"。
        use crate::server::entry::{TodoPhase, TodoStatus, TodoTask};
        let seed = vec![TodoPhase {
            name: "阶段一".into(),
            tasks: vec![TodoTask {
                content: "甲".into(),
                status: TodoStatus::Pending,
                blocker: None,
            }],
        }];
        let mut t = BuiltinTools::new(std::env::temp_dir()).with_todo(seed);
        let call = crate::server::ai::types::ToolCall {
            id: "c".into(),
            kind: "function".into(),
            function: crate::server::ai::types::FunctionCall {
                name: "todo".into(),
                arguments: r#"{"op":"done","task":"不存在的一项"}"#.into(),
            },
        };
        let err = crate::server::agent::loop_rs::ToolExecutor::execute(&mut t, &call, &mut |_| {})
            .unwrap_err();
        assert!(err.to_string().contains("不存在的一项"), "{err}");
        assert!(t.take_todo().is_none(), "有错就不该发布任何改动");
    }

    #[test]
    fn the_roster_and_the_executor_read_the_same_gate() {
        // A tool hidden from the roster must be refused by the executor, and a
        // tool that is *not* filterable must survive any allow-list. Both
        // readers now derive from `TOOLS`, so this cannot drift — the test
        // pins the property, not the implementation.
        let mut t = BuiltinTools::new(std::env::temp_dir()).with_enabled(Some(vec!["read".into()]));
        let names: Vec<String> = t
            .definitions_for()
            .iter()
            .map(|d| d.function.name.clone())
            .collect();
        assert_eq!(names, vec!["read", "context"], "只有 read 与不可过滤的 context");

        let call = |name: &str| crate::server::ai::types::ToolCall {
            id: "c".into(),
            kind: "function".into(),
            function: crate::server::ai::types::FunctionCall {
                name: name.into(),
                arguments: "{}".into(),
            },
        };
        let err = crate::server::agent::loop_rs::ToolExecutor::execute(&mut t, &call("bash"), &mut |_| {})
            .expect_err("被 profile 关掉的工具必须拒绝");
        assert!(
            err.to_string().contains("disabled by the active profile"),
            "拒绝理由要说清是 profile: {err}"
        );
    }

    #[test]
    fn edit_hands_the_ui_a_diff_it_computed_itself() {
        // The diff is the tool's own knowledge: it knows the two sides exactly,
        // so the UI never reconstructs them from the result sentence (which is
        // just "replaced: <path>"). This is the payload the renderer draws.
        let d = tempdir("diff_details");
        let path = d.join("f.txt");
        std::fs::write(&path, "keep\nold line\nkeep2\n").unwrap();
        let args = parse_edit_args(
            &serde_json::json!({"path": path.display().to_string(), "old": "old line", "new": "new line"})
                .to_string(),
        )
        .unwrap();
        let out = edit(&d, &licensed_zones(&d), &args).unwrap();
        assert!(out.text.contains("replaced"), "{}", out.text);
        let details = out.details.expect("edit 必须交 details");
        assert_eq!(details["kind"], "diff");
        assert_eq!(details["deletions"], serde_json::json!(["old line"]));
        assert_eq!(details["insertions"], serde_json::json!(["new line"]));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn mass_edit_hands_over_the_lines_it_replaced() {
        let d = tempdir("mass_details");
        let path = d.join("f.txt");
        std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
        let args = parse_mass_edit_args(
            &serde_json::json!({"path": path.display().to_string(), "edits": [
                {"line": 1, "text": "ONE"}, {"line": 3, "text": "THREE"}
            ]})
            .to_string(),
        )
        .unwrap();
        let out = mass_edit(&d, &licensed_zones(&d), &args).unwrap();
        let details = out.details.expect("mass_edit 必须交 details");
        assert_eq!(details["kind"], "diff");
        // Largest line first internally, but the diff reads in file order.
        assert_eq!(details["deletions"], serde_json::json!(["three", "one"]));
        assert_eq!(details["insertions"], serde_json::json!(["THREE", "ONE"]));
        let _ = std::fs::remove_dir_all(&d);
    }

    #[cfg(feature = "web")]
    #[test]
    fn web_tools_are_in_the_roster_with_the_feature() {
        let defs = BuiltinTools::definitions();
        let names: Vec<&str> = defs.iter().map(|d| d.function.name.as_str()).collect();
        for present in ["fetch", "browser", "search"] {
            assert!(
                names.contains(&present),
                "{present} 必须在名册里: {names:?}"
            );
        }
    }

    #[cfg(not(feature = "web"))]
    #[test]
    fn web_tools_are_absent_without_the_feature() {
        // A build without `web` must not advertise tools it cannot run, and a
        // hallucinated call must say so — silence would make the model believe
        // the fetch had happened.
        let defs = BuiltinTools::definitions();
        let names: Vec<&str> = defs.iter().map(|d| d.function.name.as_str()).collect();
        for absent in ["fetch", "browser", "search"] {
            assert!(!names.contains(&absent), "{absent} 不该在名册里: {names:?}");
        }
        let mut t = BuiltinTools::new(std::env::temp_dir());
        let err = t
            .execute(&ToolCall::new("c1", "fetch", "{}"))
            .unwrap_err()
            .to_string();
        assert!(err.contains("not compiled in"), "{err}");
    }

    #[test]
    fn edit_replaces_the_single_occurrence() {
        let d = tempdir("edit-single");
        let f = d.join("a.txt");
        std::fs::write(&f, "红鲤鱼与绿鲤鱼\n").unwrap();

        let args = EditArgs {
            path: "a.txt".into(),
            old: "红鲤鱼".into(),
            new: "绿鲤鱼".into(),
        };
        let out = edit(&d, &licensed_zones(&d), &args).unwrap();
        assert!(out.text.contains("replaced"));
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "绿鲤鱼与绿鲤鱼\n");
    }

    #[test]
    fn edit_rejects_ambiguous_match() {
        // The ambiguous-match scenario: two identical occurrences, no way to choose.
        // Refusing is correct behavior — silently replacing the first or last is a guess.
        let d = tempdir("edit-ambiguous");
        let f = d.join("a.txt");
        std::fs::write(&f, "红鲤鱼与红鲤鱼\n").unwrap();

        let args = EditArgs {
            path: "a.txt".into(),
            old: "红鲤鱼".into(),
            new: "绿鲤鱼".into(),
        };
        let err = format!("{:#}", edit(&d, &licensed_zones(&d), &args).unwrap_err());
        assert!(err.contains("2 times"), "{err}");
        assert!(err.contains("mass_edit"), "要把出路指给模型: {err}");
        assert_eq!(
            std::fs::read_to_string(&f).unwrap(),
            "红鲤鱼与红鲤鱼\n",
            "不能改"
        );
    }

    #[test]
    fn edit_requires_exact_match() {
        let d = tempdir("edit-nomatch");
        std::fs::write(d.join("a.txt"), "红鲤鱼\n").unwrap();
        let args = EditArgs {
            path: "a.txt".into(),
            old: "红鲤渔".into(), // 错别字
            new: "绿鲤鱼".into(),
        };
        let err = format!("{:#}", edit(&d, &licensed_zones(&d), &args).unwrap_err());
        assert!(err.contains("not found"), "{err}");
    }

    #[test]
    fn edit_accepts_relative_paths_with_dot_dot() {
        let d = tempdir("edit-relpath");
        std::fs::create_dir_all(d.join("sub")).unwrap();
        std::fs::write(d.join("a.txt"), "hello\n").unwrap();

        // From sub/, reference the parent file via ../
        let args = EditArgs {
            path: "../a.txt".into(),
            old: "hello".into(),
            new: "world".into(),
        };
        let cwd = d.join("sub");
        edit(&cwd, &licensed_zones(&cwd), &args).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "world\n");
    }

    #[test]
    fn edit_refuses_missing_file_and_directory() {
        let d = tempdir("edit-guards");
        std::fs::create_dir_all(d.join("sub")).unwrap();

        let args = EditArgs {
            path: "nope.txt".into(),
            old: "a".into(),
            new: "b".into(),
        };
        assert!(edit(&d, &licensed_zones(&d), &args).is_err());

        let args = EditArgs {
            path: "sub".into(), // 是目录
            old: "a".into(),
            new: "b".into(),
        };
        let err = format!("{:#}", edit(&d, &licensed_zones(&d), &args).unwrap_err());
        assert!(err.contains("directory"), "{err}");
    }

    #[test]
    fn mass_edit_replaces_whole_lines() {
        let d = tempdir("mass-basic");
        let f = d.join("a.txt");
        std::fs::write(&f, "红鲤鱼\n绿鲤鱼\n红鲤鱼\n").unwrap();

        let args = MassEditArgs {
            path: "a.txt".into(),
            edits: vec![
                LineEdit {
                    line: 1,
                    text: "绿鲤鱼".into(),
                },
                LineEdit {
                    line: 3,
                    text: "绿鲤鱼".into(),
                },
            ],
        };
        mass_edit(&d, &licensed_zones(&d), &args).unwrap();
        // Whole-line replacement: overwrite regardless of the previous content
        assert_eq!(
            std::fs::read_to_string(&f).unwrap(),
            "绿鲤鱼\n绿鲤鱼\n绿鲤鱼\n"
        );
    }

    #[test]
    fn mass_edit_lines_are_stable_when_editing_from_the_bottom() {
        // Key property: applying from the largest line number down keeps earlier line numbers stable
        let d = tempdir("mass-stable");
        let f = d.join("a.txt");
        std::fs::write(&f, "a\nb\nc\nd\ne\n").unwrap();

        let args = MassEditArgs {
            path: "a.txt".into(),
            edits: vec![
                LineEdit {
                    line: 5,
                    text: "E".into(),
                },
                LineEdit {
                    line: 2,
                    text: "B".into(),
                },
            ],
        };
        mass_edit(&d, &licensed_zones(&d), &args).unwrap();
        assert_eq!(std::fs::read_to_string(&f).unwrap(), "a\nB\nc\nd\nE\n");
    }

    #[test]
    fn mass_edit_rejects_duplicate_and_out_of_range_lines() {
        let d = tempdir("mass-guards");
        let f = d.join("a.txt");
        std::fs::write(&f, "a\nb\n").unwrap();

        let args = MassEditArgs {
            path: "a.txt".into(),
            edits: vec![
                LineEdit {
                    line: 2,
                    text: "x".into(),
                },
                LineEdit {
                    line: 2,
                    text: "y".into(),
                },
            ],
        };
        let err = format!("{:#}", mass_edit(&d, &licensed_zones(&d), &args).unwrap_err());
        assert!(err.contains("twice"), "{err}");

        let args = MassEditArgs {
            path: "a.txt".into(),
            edits: vec![LineEdit {
                line: 9,
                text: "x".into(),
            }],
        };
        let err = format!("{:#}", mass_edit(&d, &licensed_zones(&d), &args).unwrap_err());
        assert!(err.contains("9"), "{err}");
    }

    #[test]
    fn mass_edit_parses_both_argument_shapes() {
        let a = parse_mass_edit_args(r#"{"path":"p","edits":[{"line":3,"text":"t"}]}"#).unwrap();
        assert_eq!(
            a.edits,
            vec![LineEdit {
                line: 3,
                text: "t".into()
            }]
        );

        let b = parse_mass_edit_args(r#"{"path":"p","lines":[1,2],"text":"x"}"#).unwrap();
        assert_eq!(
            b.edits,
            vec![
                LineEdit {
                    line: 1,
                    text: "x".into()
                },
                LineEdit {
                    line: 2,
                    text: "x".into()
                },
            ]
        );

        assert!(parse_mass_edit_args(r#"{"path":"p"}"#).is_err());
        assert!(parse_mass_edit_args(r#"{"path":"p","edits":[{"line":0,"text":"t"}]}"#).is_err());
    }

    #[test]
    fn malformed_arguments_report_readable_errors() {
        // Models emit bad JSON routinely; errors must be readable enough for self-correction
        let err = format!("{:#}", parse_edit_args("{不是json").unwrap_err());
        assert!(err.contains("JSON"), "{err}");

        let err = format!("{:#}", parse_edit_args(r#"{"path":"a"}"#).unwrap_err());
        assert!(err.contains("old"), "{err}");
    }

    #[test]
    fn executor_dispatches_by_name_and_reports_unknown() {
        let d = tempdir("exec");
        std::fs::write(d.join("a.txt"), "红鲤鱼\n").unwrap();
        let mut t = BuiltinTools::new(d.clone());

        let call = ToolCall::new(
            "c1",
            "edit",
            json!({
                "path": "a.txt", "old": "红鲤鱼", "new": "绿鲤鱼"
            })
            .to_string(),
        );
        let out = t.execute(&call, &mut |_| {}).unwrap();
        assert!(out.text.contains("replaced"));

        let bad = ToolCall::new("c2", "不存在的工具", "{}");
        let err = format!("{:#}", t.execute(&bad, &mut |_| {}).unwrap_err());
        assert!(err.contains("unknown tool"), "{err}");
    }

    #[test]
    fn definitions_carry_the_core_tools() {
        // The core roster (web tools are conditional on the `web` feature and
        // are asserted separately, see web_tools_are_*).
        let defs = BuiltinTools::definitions();
        let names: Vec<&str> = defs.iter().map(|d| d.function.name.as_str()).collect();
        for core in ["read", "edit", "write", "cd", "bash", "mass_edit", "context"] {
            assert!(names.contains(&core), "{core} 必须在名册里: {names:?}");
        }
        // `context` is last: it is the compressed session's escape hatch.
        assert_eq!(names.last(), Some(&"context"));
        // The schema must declare required fields, or the model omits arguments
        for d in &defs {
            assert!(d.function.parameters.get("required").is_some());
        }
    }

    fn hist() -> Vec<crate::server::entry::Entry> {
        use crate::server::entry::Entry;
        vec![
            Entry::User {
                content: "帮我跑 tree".into(),
            },
            Entry::ToolRequest {
                call_id: "c1".into(),
                name: "bash".into(),
                args: "{}".into(),
                intent: "列出目录".into(),
                text: String::new(),
                first: true,
            },
            // The artifact placeholder embeds `#1`.
            Entry::ToolResult {
                call_id: "c1".into(),
                name: "bash".into(),
                ok: true,
                result: "[工具输出共 50000 行 / 640KB，过大已存为巨物 #1（bash）。]".into(),
                details: None,
                duration_ms: 0,
            },
            Entry::ToolRequest {
                call_id: "c2".into(),
                name: "read".into(),
                args: "{\"path\":\"a.rs\"}".into(),
                intent: "读文件".into(),
                text: String::new(),
                first: true,
            },
            Entry::ToolResult {
                call_id: "c2".into(),
                name: "read".into(),
                ok: true,
                result: "1: fn main() {".into(),
                details: None,
                duration_ms: 0,
            },
        ]
    }

    #[test]
    fn context_finds_artifact_anchor() {
        let h = hist();
        let args = ContextArgs {
            anchor: "#1".into(),
            before: 10,
            after: 10,
            include_user: false,
            include_results: false,
        };
        let out = context_query(&h, &[], &args).unwrap();
        assert!(out.text.contains("调用 bash"), "{}", out.text);
        assert!(out.text.contains("结果 bash"), "{}", out.text);
        // include_results=false must NOT embed the full result text
        assert!(!out.text.contains("50000 行"), "{}", out.text);
    }

    #[test]
    fn context_includes_results_and_users_when_asked() {
        let h = hist();
        let args = ContextArgs {
            anchor: "c2".into(),
            before: 10,
            after: 10,
            include_user: true,
            include_results: true,
        };
        let out = context_query(&h, &[(1, "/tmp/elsewhere".into())], &args).unwrap();
        assert!(out.text.contains("fn main()"), "{}", out.text);
        assert!(out.text.contains("帮我跑 tree"), "{}", out.text);
    }

    #[test]
    fn context_unknown_anchor_is_an_error() {
        let h = hist();
        let args = ContextArgs {
            anchor: "#9".into(),
            before: 1,
            after: 1,
            include_user: false,
            include_results: false,
        };
        let err = format!("{:#}", context_query(&h, &[], &args).unwrap_err());
        assert!(err.contains("#9"), "{err}");
    }

    #[test]
    fn line_count_matches_for_trailing_newline() {
        // Line-count edge: with or without a trailing newline, lines() counts the same
        assert_eq!("a\nb\n".lines().count(), 2);
        assert_eq!("a\nb".lines().count(), 2);
    }

    #[test]
    fn cwd_is_respected() {
        let d = tempdir("cwd");
        std::fs::write(d.join("a.txt"), "x\n").unwrap();
        let args = EditArgs {
            path: "a.txt".into(),
            old: "x".into(),
            new: "y".into(),
        };
        edit(Path::new(&d), &licensed_zones(&d), &args).unwrap();
        assert_eq!(std::fs::read_to_string(d.join("a.txt")).unwrap(), "y\n");
    }

    #[test]
    fn bash_rejects_out_of_zone_and_disasters() {
        let cwd = std::env::temp_dir();
        for bad in [
            "rm -rf /",
            "rm -rf ~",
            "rm ~/notes.txt",
            "rm -rf /etc",
            "rm ../../outside.txt",
            "dd if=/dev/zero of=/dev/sda",
            "bash -c 'exec 3<>/dev/tcp/10.0.0.1/4242'",
            "ls\nrm -rf /",
            "",
        ] {
            assert!(bash_with_stop(&cwd, bad, None, T, None, L, &mut |_| {}).is_err(), "应拒绝: {bad:?}");
        }
    }

    #[test]
    fn read_stops_at_the_budget_and_says_how_to_continue() {
        // The conversation keeps a bounded slice of a big file: the head, plus
        // the offset that continues. No artifact (the file is not output we
        // produced), no unbounded read.
        let d = tempdir("read_cap");
        let p = d.join("big.txt");
        let body: String = (1..=50).map(|i| format!("line{i}\n")).collect();
        std::fs::write(&p, &body).unwrap();

        let limits = crate::server::ai::config::OutputLimits {
            max_lines: 10,
            max_bytes: 1 << 20,
        };
        let args = parse_read_args(&serde_json::json!({"path": p.display().to_string()}).to_string())
            .unwrap();
        let out = read(&d, &licensed_zones(&d), limits, &args).unwrap();
        assert!(out.text.contains("line1\n") || out.text.contains("line1"), "{}", out.text);
        assert!(!out.text.contains("line11"), "不该越过上限: {}", out.text);
        assert!(
            out.text.contains("offset=11"),
            "必须告诉模型从哪继续: {}",
            out.text
        );
        let details = out.details.unwrap();
        assert_eq!(details["lines"], 10);
        assert_eq!(details["more"], true);

        // The continuation reads the next slice, numbered from where it is.
        let cont = parse_read_args(
            &serde_json::json!({"path": p.display().to_string(), "offset": 11}).to_string(),
        )
        .unwrap();
        let out = read(&d, &licensed_zones(&d), limits, &cont).unwrap();
        assert!(out.text.contains("line11"), "{}", out.text);
        assert!(!out.text.contains("line1\n"), "续读不该重放开头: {}", out.text);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn read_says_so_when_the_byte_budget_stops_it_mid_file() {
        // One enormous line is the case the byte bound exists for; the notice
        // must not pretend it knows the total when it never scanned to EOF.
        let d = tempdir("read_bytes");
        let p = d.join("wide.txt");
        std::fs::write(&p, format!("{}\nsecond\n", "x".repeat(500))).unwrap();
        let limits = crate::server::ai::config::OutputLimits {
            max_lines: 100,
            max_bytes: 64,
        };
        let args = parse_read_args(&serde_json::json!({"path": p.display().to_string()}).to_string())
            .unwrap();
        let out = read(&d, &licensed_zones(&d), limits, &args).unwrap();
        assert!(out.text.contains("offset=2"), "{}", out.text);
        assert!(
            out.text.contains("总行数未知"),
            "字节上限截断时必须承认没读到尾: {}",
            out.text
        );
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn file_tools_stay_inside_the_licensed_zones() {
        // The boundary the whole refactor exists for: reading/writing a path
        // outside the workspace and temp is refused *before* any I/O, and the
        // refusal names both licensed zones so the model can correct itself.
        let d = tempdir("zones_ws");
        let outside = home_dir().join(".mypi_outside_probe");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "nope").unwrap();

        let zones = licensed_zones(&d);
        let r = parse_read_args(
            &serde_json::json!({"path": outside.join("secret.txt").display().to_string()})
                .to_string(),
        )
        .unwrap();
        let err = read(&d, &zones, crate::server::ai::config::OutputLimits::default(), &r)
            .unwrap_err();
        assert!(
            format!("{err:#}").contains("outside the workspace"),
            "越界读必须拒绝: {err:#}"
        );
        let w = parse_write_args(
            &serde_json::json!({"path": outside.join("new.txt").display().to_string(), "content": "x"})
                .to_string(),
        )
        .unwrap();
        assert!(write(&d, &zones, &w).is_err(), "越界写必须拒绝");
        assert!(!outside.join("new.txt").exists(), "拒绝就该一个字节都没写");

        // The temp dir IS licensed: the user's scratch work lives there.
        let scratch = std::env::temp_dir().join("mypi_zone_probe.txt");
        let w = parse_write_args(
            &serde_json::json!({"path": scratch.display().to_string(), "content": "ok"}).to_string(),
        )
        .unwrap();
        assert!(write(&d, &zones, &w).is_ok(), "/tmp 必须可用");
        assert_eq!(std::fs::read_to_string(&scratch).unwrap(), "ok");

        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&outside);
        let _ = std::fs::remove_file(&scratch);
    }

    #[test]
    fn write_creates_then_overwrites_and_says_which_it_did() {
        let d = tempdir("write_tool");
        let p = d.join("f.txt");
        let zones = licensed_zones(&d);

        let first = parse_write_args(
            &serde_json::json!({"path": p.display().to_string(), "content": "one\n"}).to_string(),
        )
        .unwrap();
        let out = write(&d, &zones, &first).unwrap();
        assert!(out.text.contains("已新建"), "{}", out.text);
        assert_eq!(out.details.unwrap()["created"], true);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "one\n");

        // Same path again: the whole point of `write` is that it replaces.
        let second = parse_write_args(
            &serde_json::json!({"path": p.display().to_string(), "content": "two\n"}).to_string(),
        )
        .unwrap();
        let out = write(&d, &zones, &second).unwrap();
        assert!(out.text.contains("已覆盖"), "{}", out.text);
        let details = out.details.unwrap();
        assert_eq!(details["created"], false);
        assert_eq!(details["bytes"], 4);
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "two\n", "必须是覆盖不是追加");

        // A missing parent directory is refused with a fix in the message.
        let missing = parse_write_args(
            &serde_json::json!({"path": d.join("no/such/f.txt").display().to_string(), "content": "x"})
                .to_string(),
        )
        .unwrap();
        let err = write(&d, &zones, &missing).unwrap_err();
        assert!(format!("{err:#}").contains("mkdir -p"), "{err:#}");
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn cd_moves_the_license_zone_within_the_workspace_only() {
        let d = std::env::temp_dir().join("mypi_zone_a");
        let inner = d.join("inner");
        let other = home_dir().join(".mypi_zone_b");
        let _ = std::fs::create_dir_all(&d);
        let _ = std::fs::create_dir_all(&inner);
        let _ = std::fs::create_dir_all(&other);

        let mut t = BuiltinTools::new(d.clone());
        // Into a subdirectory: the zone follows, and the delete is licensed.
        t.tool_cd(inner.to_str().unwrap()).unwrap();
        assert_eq!(t.cwd, inner.canonicalize().unwrap());
        let victim = inner.join("victim.txt");
        std::fs::write(&victim, "x").unwrap();
        assert!(
            bash_with_stop(
                &t.cwd.clone(),
                &format!("rm {}", victim.display()),
                None,
                T,
                None,
                L,
                &mut |_| {}
            )
            .is_ok(),
            "子目录仍在工作区内，应放行"
        );

        // Out of the workspace: refused. `cd` is the only way the model moves
        // the zone, so an unbounded cd would make every file boundary advisory.
        let err = t.tool_cd(other.to_str().unwrap()).unwrap_err();
        assert!(
            format!("{err:#}").contains("outside the workspace"),
            "越界 cd 必须拒绝: {err:#}"
        );
        assert_eq!(t.cwd, inner.canonicalize().unwrap(), "拒绝后 cwd 不该变");

        let _ = std::fs::remove_dir_all(&d);
        let _ = std::fs::remove_dir_all(&other);
    }

    #[test]
    fn bash_real_shell_inside_zone() {
        let cwd = std::env::temp_dir();
        // Pipes/redirects run free inside the zone.
        let out = bash_with_stop(&cwd, "echo hello | tr a-z A-Z", None, T, None, L, &mut |_| {}).unwrap();
        assert!(out.text.contains("HELLO"), "管道可用: {:?}", out.text);
        // grep with no match -> exit 1, reported verbatim
        let out = bash_with_stop(&cwd, "grep zzzz /dev/null", None, T, None, L, &mut |_| {}).unwrap();
        assert!(out.text.contains("[exit"), "非零退出要标注: {:?}", out.text);
        // High tier warns but executes.
        let out = bash_with_stop(&cwd, "echo ok", None, T, None, L, &mut |_| {}).unwrap().text;
        assert_eq!(out, "ok");
    }

    #[test]
    fn cd_tool_moves_directory_and_writes_slot() {
        let tmp = std::env::temp_dir();
        let start = tmp.clone();
        let slot = std::sync::Arc::new(std::sync::RwLock::new(start.clone()));
        let mut t = BuiltinTools::new(start.clone()).with_cwd_slot(slot.clone());
        let deep = tmp.join("mypi-cdtest");
        std::fs::create_dir_all(&deep).unwrap();
        let arg = format!("{{\"path\": \"{}\"}}", deep.display());
        let call = ToolCall {
            id: "1".into(),
            kind: "function".into(),
            function: crate::server::ai::types::FunctionCall {
                name: "cd".into(),
                arguments: arg,
            },
        };
        let r = crate::server::agent::loop_rs::ToolExecutor::execute(&mut t, &call, &mut |_| {}).unwrap();
        assert!(r.text.contains("changed to"), "结果说明: {:?}", r.text);
        assert_eq!(*slot.read().unwrap(), deep, "共享槽被写回");
        let _ = std::fs::remove_dir_all(&deep);
    }

    #[test]
    fn a_long_command_reports_while_it_is_still_running() {
        // The whole point of progress: a command that prints for a while must
        // be visibly alive. The first chunk has to arrive *during* the call —
        // a tool that only speaks at the end is exactly the frozen card this
        // exists to avoid.
        let cwd = std::env::temp_dir();
        let mut seen: Vec<(std::time::Instant, String)> = Vec::new();
        let started = std::time::Instant::now();
        let out = bash_with_stop(
            &cwd,
            "for i in 1 2 3; do echo 第$i行; sleep 0.25; done",
            None,
            T,
            None,
            L,
            &mut |p| {
                let ToolProgress::Output(text) = p;
                seen.push((std::time::Instant::now(), text));
            },
        )
        .unwrap();
        assert!(out.text.contains("第3行"), "{}", out.text);
        let (at, text) = seen.first().expect("中途必须报过进度");
        assert!(text.contains("第1行"), "第一块进度就该有第一行: {text:?}");
        assert!(
            at.duration_since(started) < std::time::Duration::from_millis(600),
            "进度必须是跑的时候来的，不是结束才给：{:?}",
            at.duration_since(started)
        );
        // …and it was still running when that arrived (the command sleeps 750ms).
        assert!(
            started.elapsed() > at.duration_since(started),
            "进度不能是结束后才补发的"
        );
    }

    #[test]
    fn bash_timeout_kills_a_hung_command() {
        // The whole point: a command that never returns must not block the
        // turn thread forever. Cap at 1s and prove it comes back — with a
        // marker, not a hang — in well under the command's own runtime.
        let cwd = std::env::temp_dir();
        let t0 = std::time::Instant::now();
        let out =
            bash_with_stop(&cwd, "sleep 30", None, std::time::Duration::from_secs(1), None, L, &mut |_| {}).unwrap();
        let elapsed = t0.elapsed();
        assert!(
            elapsed < std::time::Duration::from_secs(15),
            "超时必须生效，实际耗时 {elapsed:?}"
        );
        assert!(out.text.contains("timeout"), "超时要标注: {:?}", out.text);
    }

    #[test]
    fn bash_timeout_leaves_a_pipe_grandchild_nothing_to_hang_on() {
        // The subtle case: `a | b` — `b` holds the stdout pipe's write end.
        // Killing only bash would leave `b` alive and the reader blocked on
        // a pipe that never EOFs. The group signal must take `b` too, so
        // this returns promptly with a timeout marker.
        let cwd = std::env::temp_dir();
        let t0 = std::time::Instant::now();
        let out = bash_with_stop(
            &cwd,
            "sleep 30 | cat",
            None,
            std::time::Duration::from_secs(1),
            None,
            L,
            &mut |_| {},
        )
        .unwrap();
        assert!(
            t0.elapsed() < std::time::Duration::from_secs(15),
            "管道孙进程也要被收掉，实际耗时 {:?}",
            t0.elapsed()
        );
        assert!(out.text.contains("timeout"), "超时要标注: {:?}", out.text);
    }

    #[test]
    fn an_external_stop_kills_the_group_before_the_deadline() {
        // The daemon-level stop: arm the session flag (the front-end Esc
        // path does this) and prove the group dies promptly — well before
        // the 60s deadline.
        use std::sync::atomic::AtomicBool;
        let cwd = std::env::temp_dir();
        let flag = std::sync::Arc::new(AtomicBool::new(false));
        let setter = flag.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(300));
            setter.store(true, std::sync::atomic::Ordering::Relaxed);
        });
        let t0 = std::time::Instant::now();
        let out = bash_with_stop(&cwd, "sleep 60", None, std::time::Duration::from_secs(60), Some(flag), L, &mut |_| {}).unwrap();
        assert!(
            t0.elapsed() < std::time::Duration::from_secs(10),
            "外部停止必须即时生效，实际耗时 {:?}",
            t0.elapsed()
        );
        assert!(out.text.contains("interrupted/timeout"), "停止要标注: {:?}", out.text);
    }
}
