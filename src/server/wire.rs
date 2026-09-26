//! The daemon ↔ front end wire protocol: message types and the line-delimited
//! JSON codec.
//!
//! Shape (SERVER.md §2–§3): one JSON object per line over a unix stream socket;
//! the first line must be `ClientMsg::Hello`, answered by `HelloOk` or
//! `Error{proto_mismatch}`. The wire carries **what a front end draws** —
//! [`Entry`] verbatim, a [`StreamView`]-shaped snapshot, a status line — never
//! internal `SessionEvent`s.

use crate::server::entry::Entry;
use crate::server::events::{LiveActivity, RunState};
use crate::server::session::Replay;

/// Wire protocol version. Bump on any incompatible change to the message
/// shapes below; a mismatching `hello` is refused before anything else runs.
pub const PROTO_VERSION: u32 = 1;

/// Hard cap on one encoded line. A big transcript really does reach a few MB,
/// so this is not a 64 KiB joke value; it only exists to refuse garbage or
/// hostile peers before they eat memory.
pub const MAX_LINE_BYTES: usize = 32 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// Front end → daemon.
///
/// There is deliberately **no `new_session`**: a fresh front end starts in a
/// draft state (no session, no id, zero database rows) and the daemon creates
/// the session on the first `submit` (SERVER.md §1).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMsg {
    /// Handshake. The only message accepted before `hello_ok` arrived.
    Hello { proto: u32, client: String },
    /// Attach to an existing session (resume has no other semantics).
    Attach { id: i64 },
    /// Leave the current session. The daemon stops its round when the last
    /// watcher leaves (SERVER.md §4).
    Detach,
    /// List sessions (picker, `mypi sessions`).
    /// Delete a stored session and everything hanging off it (entries, cwd
    /// history, artifacts, round headers). Refused while the session is open
    /// in memory — the picker's list is the only place this is offered, and a
    /// live session's row is being written to.
    DeleteSession { id: i64 },
    /// Ask for the resume picker's list. `under` narrows it to sessions that
    /// ever ran in that directory (project scope); `None` = all of them.
    ListSessions {
        #[serde(default)]
        under: Option<String>,
    },
    /// One session's stored round headers (audit / reproduce).
    ListRounds { id: i64 },
    /// Rebuild one stored round's request from the database (read-only).
    Replay { id: i64, round: i64 },
    /// Start a round. In the draft state this creates the session row first
    /// and pushes `Attached` back before anything else.
    Submit { text: String },
    /// Interrupt the running round of the attached session. Valid any time;
    /// it is the only message that is read even mid-stream.
    Interrupt,
    /// Run a slash command. The front end only **recognizes** it (the table
    /// travels down in [`ServerMsg::Commands`]); the session owns the meaning,
    /// so the dispatch lives on this side of the wire.
    Command { name: String, args: String },
    /// Fetch the in-memory narration ring (retries, timeouts).
    Logs { limit: usize },
    /// Finish the round in progress and close cleanly.
    Quit,
}

/// One slash command, as a front end sees it: metadata only — what it is called,
/// what it does, and whether the front end has to handle it itself.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct CommandInfo {
    pub name: String,
    #[serde(default)]
    pub aliases: Vec<String>,
    pub detail: String,
    /// Argument shape, as a string: `none` / `model_id` / `path` / `text`.
    /// A string rather than an enum so an older front end can still render a
    /// command it has never heard of.
    pub args: String,
    /// `session` = the daemon runs it; `local` = the front end owns it.
    pub scope: String,
}

impl From<&crate::server::commands::CommandSpec> for CommandInfo {
    fn from(c: &crate::server::commands::CommandSpec) -> Self {
        use crate::server::commands::{ArgKind, Scope};
        Self {
            name: c.name.to_string(),
            aliases: c.aliases.iter().map(|a| a.to_string()).collect(),
            detail: c.detail.to_string(),
            args: match c.args {
                ArgKind::None => "none",
                ArgKind::ModelId => "model_id",
                ArgKind::ProfileName => "profile_name",
                ArgKind::Path => "path",
                ArgKind::Text => "text",
            }
            .to_string(),
            scope: match c.scope {
                Scope::Session => "session",
                Scope::Local => "local",
            }
            .to_string(),
        }
    }
}

/// Every command, ready to ship.
pub fn command_table() -> Vec<CommandInfo> {
    crate::server::commands::COMMANDS
        .iter()
        .map(CommandInfo::from)
        .collect()
}

/// Daemon → front end.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMsg {
    /// Handshake accepted; no session is attached yet. Carries the slash-command
    /// **table**: a front end offers completion and help from it, and never
    /// keeps a second copy that could drift (there used to be one in the TUI).
    HelloOk {
        proto: u32,
        commands: Vec<CommandInfo>,
    },
    /// The session became known: explicit attach, or the draft's first submit.
    Attached { session_id: i64 },
    /// Full transcript snapshot (on attach, and whenever the generation
    /// changes — branch switch, compaction, resume).
    Transcript { entries: Vec<Entry> },
    /// One new entry appended to the transcript tail.
    Entry { entry: Entry },
    /// Several new entries appended since the last frame (the daemon
    /// coalesces a burst — e.g. a finished tool run — into one message).
    EntryMany { entries: Vec<Entry> },
    /// The in-flight streaming snapshot, merged at most every ~33ms.
    Stream {
        active: bool,
        text: String,
        reasoning: String,
        reasoning_done: bool,
        live: LiveActivity,
        run_state: RunState,
        /// Output of the **running** tool so far (bounded tail; see
        /// `SessionEvent::ToolProgress`). Empty when no tool is running.
        #[serde(default)]
        tool_output: String,
    },
    /// Status line values; sent when one of them changes.
    State {
        /// Model **display name** (models.yml `name`, else the id).
        model: String,
        name: Option<String>,
        cwd: String,
        spend_usd: f64,
        last_prompt_tokens: u64,
        /// Context window (models.yml) — the usage gauge's denominator.
        /// 0 = unknown: the gauge hides instead of lying.
        context_window: u64,
        busy: bool,
    },
    /// `list_sessions` answer.
    Sessions { sessions: Vec<SessionInfo> },
    /// `list_rounds` answer.
    Rounds { id: i64, rounds: Vec<RoundInfo> },
    /// `replay` answer.
    Replay { id: i64, replay: Replay },
    /// `logs` answer.
    Logs { records: Vec<LogRecord> },
    /// Protocol-level error (transport, unknown ids, bad state). A round's
    /// own errors travel as `Entry` instead.
    Error { code: ErrorCode, message: String },
}

/// Error codes the daemon can report on the wire. Kept tiny: the message body
/// carries the human text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Handshake version mismatch; the daemon disconnects after sending it.
    ProtoMismatch,
    /// `attach` / `list_rounds` / `replay` named a session that does not exist.
    NoSuchSession,
    /// `replay` named a round the session does not have.
    NoSuchRound,
    /// A session is already streaming; `submit` was refused.
    Busy,
    /// Anything else; the message text says what.
    Internal,
}

/// `sessions` row — what a picker needs, computed server-side.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SessionInfo {
    pub id: i64,
    pub name: Option<String>,
    pub started_at: String,
    pub cwd: Option<String>,
    /// First user message, verbatim — the picker's preview line, and the
    /// fallback display name for an unnamed session. `None` for an empty one.
    #[serde(default)]
    pub first_message: Option<String>,
    /// Stored bytes of this session's entries (the picker's size column).
    #[serde(default)]
    pub bytes: i64,
}

/// `rounds` row — one stored request header, minus the huge verbatim fields
/// (`system` / `tools_json`): those come back through `replay` only.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct RoundInfo {
    pub seq: i64,
    pub ts: String,
    pub model: String,
    pub protocol: String,
    pub base_url: String,
    pub max_tokens: u32,
    pub stop_reason: Option<String>,
    pub first_seq: Option<i64>,
    pub last_seq: Option<i64>,
}

/// `logs` row — mirror of `log::Record` (that type stays wire-agnostic).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct LogRecord {
    pub at_ms: u64,
    pub level: String,
    pub scope: String,
    pub text: String,
}

// ---------------------------------------------------------------------------
// Codec
// ---------------------------------------------------------------------------

/// One message → one line (newline-terminated). Infallible: every field is
/// JSON-native by construction; a failure here is a bug, so it panics.
pub fn encode(msg: &impl serde::Serialize) -> Vec<u8> {
    let mut out = serde_json::to_vec(msg).expect("wire type failed to serialize");
    out.push(b'\n');
    out
}

/// What [`decode`] found in the buffer.
#[derive(Debug, Clone, PartialEq)]
pub enum Decoded<M> {
    /// One complete message; the caller drops the consumed prefix.
    Msg(M),
    /// The buffer ended mid-line; feed it more bytes and retry.
    Partial,
}

/// Parse one line-delimited JSON message from the front of `buf`.
///
/// A malformed line (bad JSON, unknown `type`, missing fields) is an error —
/// the *connection* must drop it, but the daemon and its other clients are
/// untouched. That policy lives with the caller; this function only reports.
pub fn decode<M: serde::de::DeserializeOwned>(buf: &[u8]) -> std::io::Result<Decoded<M>> {
    let Some(end) = buf.iter().position(|&b| b == b'\n') else {
        if buf.len() > MAX_LINE_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "line exceeds MAX_LINE_BYTES",
            ));
        }
        return Ok(Decoded::Partial);
    };
    let line = &buf[..end];
    if line.len() > MAX_LINE_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "line exceeds MAX_LINE_BYTES",
        ));
    }
    let msg: M = serde_json::from_slice(line)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e.to_string()))?;
    Ok(Decoded::Msg(msg))
}

/// Bytes consumed by one complete line, if the buffer holds one.
///
/// Rescans from byte 0 on every call, so it is only suitable for a buffer
/// that is searched once. A buffer that **grows by small reads** must use
/// [`LineScanner`]: a 32 000-entry transcript snapshot is one ~39 MB line
/// arriving in 16 KiB chunks, and rescanning from zero each time re-walks
/// tens of GB (see [`LineScanner`]).
pub fn line_len(buf: &[u8]) -> Option<usize> {
    buf.iter().position(|&b| b == b'\n').map(|end| end + 1)
}

/// Incremental newline search for a buffer that grows by small reads.
///
/// [`line_len`] rescans the whole buffer on every call, which makes receiving
/// **one** big line quadratic: a 32 000-entry transcript snapshot is a single
/// ~39 MB line, so the naive scan re-walks roughly `len² / (2 × chunk)` bytes —
/// tens of GB for that snapshot, which is tens of seconds of `memchr` during
/// which the front end sits on a blank screen. This remembers where the
/// previous search gave up and only looks at what arrived since.
#[derive(Default)]
pub struct LineScanner {
    /// Index in the buffer before which no newline has been seen.
    from: usize,
}

impl LineScanner {
    /// Length of the next complete line (newline included), if one is
    /// buffered. Searches only the bytes that arrived since the last call.
    pub fn next(&mut self, buf: &[u8]) -> Option<usize> {
        if self.from > buf.len() {
            // The buffer shrank behind our back; the offset is stale.
            self.from = 0;
        }
        match buf[self.from..].iter().position(|&b| b == b'\n') {
            Some(off) => Some(self.from + off + 1),
            None => {
                self.from = buf.len();
                None
            }
        }
    }

    /// Call after consuming the line [`Self::next`] returned.
    pub fn reset(&mut self) {
        self.from = 0;
    }
}

/// Socket read timeout for a front-end connection.
///
/// This is a **poll interval, not a deadline.** A read that times out means
/// "nothing arrived yet"; every wait decides for itself how long to keep
/// polling (see [`ClientConn::wait_for_within`]). Treating it as a failure is
/// what made a slow daemon look like a dead one: a 32 000-entry `attach` used
/// to die with a bare `os error 11` before the TUI painted a single cell.
pub const READ_POLL: std::time::Duration = std::time::Duration::from_secs(15);

/// Reply deadline for `hello`. The daemon answers it without touching the
/// database, so anything slower than this is a wedged daemon.
pub const HELLO_DEADLINE: std::time::Duration = std::time::Duration::from_secs(10);

/// Reply deadline for a transcript snapshot (`attach`).
///
/// The daemon reads and serializes the whole conversation here: a few hundred
/// milliseconds for a 32 000-entry session, seconds for a pathological one.
/// Five minutes is the "something is badly wrong" line, and crossing it
/// reports a named error instead of leaking an errno to the user.
pub const SNAPSHOT_DEADLINE: std::time::Duration = std::time::Duration::from_secs(300);

/// Did this error come from the socket's poll interval expiring (as opposed
/// to a real hangup)? Both a poll expiry and a `read_timeout` expiry mean the
/// same thing to a caller: nothing yet.
pub fn is_read_timeout(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

/// A minimal front-end connection: handshake, request/reply, quit. Shared by
/// the one-shot CLI commands and (later) the TUI's reader thread. Blocking
/// only; the daemon does the async-sounding work.
pub struct ClientConn {
    stream: std::sync::Arc<std::os::unix::net::UnixStream>,
    buf: Vec<u8>,
    /// Where the last newline search gave up (see [`LineScanner`]).
    scan: LineScanner,
    /// The slash-command table the handshake carried (see [`CommandInfo`]).
    /// Kept here because the handshake is consumed inside `connect` — the
    /// front end's first chance to see it is *after* connecting.
    commands: Vec<CommandInfo>,
}

use std::io::Read;

impl ClientConn {
    /// Connect and say hello. Fails when the daemon refuses the protocol
    /// version (its error message names both versions).
    pub fn connect(path: &std::path::Path) -> std::io::Result<Self> {
        let stream = std::os::unix::net::UnixStream::connect(path)?;
        stream.set_read_timeout(Some(READ_POLL))?;
        let mut me = Self {
            stream: std::sync::Arc::new(stream),
            buf: Vec::with_capacity(4096),
            scan: LineScanner::default(),
            commands: Vec::new(),
        };
        me.request(&ClientMsg::Hello { proto: PROTO_VERSION, client: "cli".into() })?;
        let reply = me.wait_for_within(|_| true, HELLO_DEADLINE)?;
        match reply {
            ServerMsg::HelloOk { commands, .. } => {
                me.commands = commands;
                Ok(me)
            }
            ServerMsg::Error { code, message } => std::io::Result::Err(
                std::io::Error::other(format!("handshake refused ({code:?}): {message}")),
            ),
            other => std::io::Result::Err(std::io::Error::other(format!(
                "expected hello_ok, got {other:?}"
            ))),
        }
    }

    /// The command table from the handshake. Empty for a client that never
    /// handshook (a test double, or a future non-`Hello` transport).
    pub fn commands(&self) -> &[CommandInfo] {
        &self.commands
    }

    /// Send a message (fire-and-forget form; answers come via `read_msg`).
    pub fn request(&self, msg: &ClientMsg) -> std::io::Result<()> {
        use std::io::Write;
        (&*self.stream).write_all(&encode(msg))?;
        (&*self.stream).flush()
    }

    /// Blocking read of one message.
    pub fn read_msg(&mut self) -> std::io::Result<ServerMsg> {
        loop {
            if let Some(n) = self.scan.next(&self.buf) {
                let line: Vec<u8> = self.buf.drain(..n).collect();
                self.scan.reset();
                return match decode::<ServerMsg>(&line)? {
                    Decoded::Msg(m) => Ok(m),
                    Decoded::Partial => unreachable!("line_len saw a newline"),
                };
            }
            let mut tmp = [0u8; 16_384];
            let n = (&*self.stream).read(&mut tmp)?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "daemon closed the connection",
                ));
            }
            self.buf.extend_from_slice(&tmp[..n]);
        }
    }

    /// Read until a message matches `pred`; non-matching messages are
    /// dropped (the caller was not interested). For order-sensitive flows
    /// use [`ClientConn::read_msg`] directly.
    ///
    /// This is [`ClientConn::wait_for_within`] with the snapshot deadline:
    /// every pre-paint wait a front end performs is a handshake reply.
    pub fn wait_for(&mut self, pred: impl Fn(&ServerMsg) -> bool) -> std::io::Result<ServerMsg> {
        self.wait_for_within(pred, SNAPSHOT_DEADLINE)
    }

    /// Read until a message matches `pred`, giving up after `within`.
    ///
    /// The socket's read timeout is only a **poll**: a slow answer is a slow
    /// answer, not a failure. Only the deadline decides, and expiry names the
    /// wait — the caller gets "daemon 在 N 秒内没有回应" rather than a bare
    /// `EAGAIN` that reads like a crash.
    pub fn wait_for_within(
        &mut self,
        pred: impl Fn(&ServerMsg) -> bool,
        within: std::time::Duration,
    ) -> std::io::Result<ServerMsg> {
        let deadline = std::time::Instant::now() + within;
        loop {
            match self.read_msg() {
                Ok(m) if pred(&m) => return Ok(m),
                Ok(_) => {}
                Err(e) if is_read_timeout(&e) => {
                    if std::time::Instant::now() >= deadline {
                        // Name the wait in the unit that fits it: whole
                        // seconds for the real deadlines, one decimal so a
                        // sub-second deadline still says something.
                        let secs = within.as_secs_f64();
                        let named = if secs.fract() == 0.0 {
                            format!("{}", secs as u64)
                        } else {
                            format!("{secs:.1}")
                        };
                        return Err(std::io::Error::new(
                            std::io::ErrorKind::TimedOut,
                            format!("daemon 在 {named} 秒内没有回应"),
                        ));
                    }
                }
                Err(e) => return Err(e),
            }
        }
    }

    /// A handle for writing requests on the SAME connection while the
    /// reader thread blocks on `read_msg`. Unix sockets are full-duplex:
    /// daemon-side one connection has a read thread and a write thread, so
    /// writes here never interfere with reads there. The wire contract that
    /// matters is elsewhere: pushes (transcript/state/stream) arrive on the
    /// reading half regardless of which connection carried the request.
    pub fn writer(&self) -> ConnWriter {
        ConnWriter {
            stream: std::sync::Arc::clone(&self.stream),
        }
    }

    /// Send `quit` and consume the ack.
    pub fn quit(&mut self) -> std::io::Result<()> {
        self.request(&ClientMsg::Quit)?;
        let _ = self.wait_for_within(|_| true, HELLO_DEADLINE)?;
        Ok(())
    }
}

/// Write-only handle to a [`ClientConn`]'s socket. Cheap to make, cloneable;
/// lives in whichever thread issues requests.
pub struct ConnWriter {
    stream: std::sync::Arc<std::os::unix::net::UnixStream>,
}

impl ConnWriter {
    /// Send a request (fire-and-forget; answers/pushes come via the reader).
    pub fn request(&mut self, msg: &ClientMsg) -> std::io::Result<()> {
        use std::io::Write;
        (&*self.stream).write_all(&encode(msg))?;
        (&*self.stream).flush()
    }
}

#[cfg(test)]
mod tests;
