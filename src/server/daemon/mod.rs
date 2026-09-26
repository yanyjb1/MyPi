//! The daemon: one process, one unix socket, N sessions, M front ends.
//!
//! Threading (SERVER.md §6): every front-end connection gets a **read**
//! thread (blocking reads → parsed `ClientMsg`s onto the command channel) and
//! a **write** thread (blocking writes from its per-connection queue). The
//! main loop is the **only** owner of the [`SessionHub`] and the database —
//! the same single-writer discipline the TUI had. Turn threads (one per
//! round, spawned by `Session::submit`) report through session event channels
//! which the main loop drains.
//!
//! Backpressure (SERVER.md §7): a slow front end never blocks a round. The
//! write queue is bounded for coalescible `Stream` frames (merged, nothing
//! dropped) and effectively unbounded for ordered facts (`entry`, `state`,
//! replies, errors). A writer that cannot keep up at all is killed by the
//! queue cap and treated as dead.

use crate::server::hub::{SessionHub, SessionSpec};
use crate::server::wire::{
    ClientMsg, ErrorCode, LogRecord, RoundInfo, ServerMsg, SessionInfo, MAX_LINE_BYTES,
    PROTO_VERSION,
};
use crate::server::events::{Change, StreamView};
use crate::server::store::Store;
use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;

/// One front end. Kept on the main thread; only its queues cross threads.
struct Conn {
    /// The socket, kept here so dropping the conn can close it (the write
    /// thread performs the actual shutdown after draining).
    stream: Arc<UnixStream>,
    /// Everything to serialize and ship, in order. Facts are unbounded (a
    /// round produces tens, not millions); stream frames are merged instead
    /// of queued, so this queue is small in practice.
    outbox: VecDeque<ServerMsg>,
    tx: Sender<ServerMsg>,
    /// Session this front end watches (`None` = the draft state).
    attached: Option<i64>,
}

/// Which session a change belongs to and what changed. Produced by the
/// session-hub drain, consumed by the fan-out.
struct Fanout {
    /// Messages every attached front end gets for this session.
    msgs: Vec<ServerMsg>,
}

/// The daemon state machine. Owns the hub; drives everything else.
pub struct Daemon {
    hub: SessionHub,
    /// Spec source: the daemon builds sessions from config, lazily, on first
    /// submit (draft) or on attach.
    spec: SessionSpec,
    conns: BTreeMap<u64, Conn>,
    next_conn_id: u64,
    cmd_rx: Receiver<Command>,
    /// A sender clone kept for threads the daemon spawns itself (the lazy-history
    /// pager); the connection threads get their own clones from `bind`.
    cmd_tx: Sender<Command>,
    /// New connections from the accept loop, waiting to be registered.
    reg_rx: Receiver<ConnectionHandle>,
    /// The accept thread's handle, held so the thread outlives `serve` but
    /// detached at drop (it must not block process exit — see `serve`).
    _accept: std::thread::JoinHandle<()>,
    /// Session → front ends currently watching it.
    watchers: HashMap<i64, Vec<u64>>,
    /// Last shipped transcript state per (session, connection): generation,
    /// live-tail length, newest block sent. A front end attaching mid-round
    /// gets a snapshot once and only the deltas afterwards.
    shipped: HashMap<(i64, u64), Ship>,
    /// Last `StreamView` shipped per session. A `Change::Stream` only says the
    /// session touched a streaming slot — **not** that the picture changed (a
    /// delta that only moves `live`/`reasoning_done` renders identically), and
    /// every shipped snapshot costs the front end a full frame. Comparing the
    /// view — not its lengths, the tool-output tail slides at constant
    /// length — keeps the wire quiet exactly when the screen is quiet.
    last_stream: HashMap<i64, StreamView>,
    /// (会话, 连接, 方向) → 后台正在读那一段。前端连着滚会重复开口（手不会
    /// 只滚一格），没有这道闸门同一个区间会读两遍、前端收到重复内容。
    paging: HashSet<(i64, u64, bool)>,
    /// Idle-exit bookkeeping (SERVER.md §4): nothing attached, nothing busy.
    idle_since: Option<std::time::Instant>,
    idle_limit: std::time::Duration,
    shutting_down: bool,
}

enum Command {
    /// A parsed message from a front end.
    Msg { conn: u64, msg: ClientMsg },
    /// 按需历史：前端点名要的那一段读回来了（见 [`spawn_page`]）。
    ///
    /// `blocks = None` = 读失败——**什么都不回**（回空的会被当成"上面没有
    /// 了"），主循环只把闸门打开，等前端再要。
    Page {
        conn: u64,
        session: i64,
        newer: bool,
        blocks: Option<Vec<crate::server::store::Block>>,
    },
    /// The connection closed (peer hangup / read error / protocol error).
    Gone { conn: u64 },
}

/// How long the daemon waits between ticks. Short enough that a submitted
/// round starts within one tick; long enough to not burn CPU.
const TICK: std::time::Duration = std::time::Duration::from_millis(5);

/// Hard cap on a connection's fact queue before we call it dead.
const OUTBOX_CAP: usize = 10_000;

/// A connection the accept loop has set up, waiting for the main loop to
/// give it an id and start queueing messages.
struct ConnectionHandle {
    stream: Arc<UnixStream>,
    ready: Sender<u64>,
}

/// Accept forever: reader thread + writer thread per connection.
///
/// The reader is the thread that owns the fd for reading; the writer clones
/// the `Arc<UnixStream>`. Protocol errors and hangups become `Command::Gone`.
fn accept_loop(listener: std::os::unix::net::UnixListener, cmd_tx: Sender<Command>, reg_tx: Sender<ConnectionHandle>) {
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let stream = Arc::new(stream);
        let (ready_tx, ready_rx) = std::sync::mpsc::channel::<u64>();
        if reg_tx
            .send(ConnectionHandle {
                stream: stream.clone(),
                ready: ready_tx,
            })
            .is_err()
        {
            break; // daemon is shutting down
        }
        // Wait for the main loop to register us before reading: otherwise a
        // fast client's messages could arrive for an unknown conn id.
        let Ok(conn_id) = ready_rx.recv() else { continue };

        let cmd = cmd_tx.clone();
        let read_stream = stream.clone();
        std::thread::spawn(move || read_loop(read_stream, conn_id, cmd));
    }
}

/// One (session, connection) shipping ledger — see
/// [`Daemon::transcript_msgs_for`].
#[derive(Clone, Copy)]
struct Ship {
    /// 转录代：变了 = 整体快照（分支跳转 / 补齐 / 压缩重排）。
    generation: u64,
    /// 已经发到前端的那截活尾巴有多长（前端的 live 就是这一段）。
    live_len: usize,
    /// 已经发过的最新块 id（0 = 一块都没发过）。
    block_id: i64,
}

/// 一次补发最多带几块（落盘补差；正常一回合也就几块）。
const NEW_BLOCKS_MAX: usize = 512;

/// 库里的一块 → 线上的一块。
fn wire_block(b: crate::server::store::Block) -> crate::server::wire::WireBlock {
    crate::server::wire::WireBlock {
        id: b.id,
        entries: b.entries,
    }
}

/// 按需历史：为**一次**请求读一段（更老或更新）。
///
/// 一次读在 32k 块上是几十毫秒——放在主循环里会卡住**所有**会话，所以它跑在
/// 自己的线程上，读完丢回命令队列。请求**点名边界 id**（`before` / `after`），
/// 所以服务端没有游标：同一会话上几个前端各看各的窗口，互不干扰；一页也只回
/// 给开口的那条连接。
///
/// 读失败**什么都不回**（不是"没有更老的了"——那会让前端闩死）。主循环把闸门
/// 打开，前端下一帧照样会再要一次。
fn spawn_page(
    db: std::path::PathBuf,
    conn: u64,
    session: i64,
    edge: i64,
    count: usize,
    newer: bool,
    cmd_tx: Sender<Command>,
) {
    std::thread::spawn(move || {
        let store = Store::open(&db);
        let blocks = store.ok().and_then(|st| {
            if newer {
                st.load_after(session, edge, count).ok()
            } else {
                st.load_before(session, edge, count).ok().map(|(b, _)| b)
            }
        });
        let _ = cmd_tx.send(Command::Page {
            conn,
            session,
            newer,
            blocks,
        });
    });
}

/// One connection's read half: parse lines into `ClientMsg`s.
fn read_loop(stream: Arc<UnixStream>, conn: u64, cmd_tx: Sender<Command>) {
    let mut buf: Vec<u8> = Vec::with_capacity(4096);
    let mut tmp = [0u8; 8192];
    let mut reader = ReadHalf(stream);
    // Incremental: `line_len` would rescan the whole buffer per read.
    let mut scan = crate::server::wire::LineScanner::default();
    loop {
        // Frames first: bytes may carry several messages.
        while let Some(n) = scan.next(&buf) {
            let line: Vec<u8> = buf.drain(..n).collect();
            scan.reset();
            match crate::server::wire::decode::<ClientMsg>(&line) {
                Ok(crate::server::wire::Decoded::Msg(msg)) => {
                    if cmd_tx.send(Command::Msg { conn, msg }).is_err() {
                        return;
                    }
                }
                Ok(crate::server::wire::Decoded::Partial) => unreachable!("line_len saw a newline"),
                Err(e) => {
                    // Bad line: refuse this connection only (SERVER.md §2).
                    let _ = cmd_tx.send(Command::Gone { conn });
                    let _ = e;
                    return;
                }
            }
        }
        match reader.read(&mut tmp) {
            Ok(0) => {
                let _ = cmd_tx.send(Command::Gone { conn });
                return;
            }
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if buf.len() > MAX_LINE_BYTES {
                    let _ = cmd_tx.send(Command::Gone { conn });
                    return;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => {
                let _ = cmd_tx.send(Command::Gone { conn });
                return;
            }
        }
    }
}

/// One connection's write half: serialize from the queue until the channel
/// closes or the socket breaks. Slow here never touches the round threads.
fn write_loop(stream: Arc<UnixStream>, rx: Receiver<ServerMsg>) {
    let mut writer = std::io::BufWriter::new(WriteHalf { stream });
    for msg in rx {
        let bytes = crate::server::wire::encode(&msg);
        if writer.write_all(&bytes).is_err() || writer.flush().is_err() {
            return; // EPIPE etc: the reader thread will notice too
        }
    }
}

/// Split an `Arc<UnixStream>` into a write half. `UnixStream` implements
/// `Write` through `&self`, so a wrapper around the clone is all it takes.
struct WriteHalf {
    stream: Arc<UnixStream>,
}
impl Write for WriteHalf {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        (&*self.stream).write(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        (&*self.stream).flush()
    }
}

/// The read half: same trick, `Read` through `&self`.
struct ReadHalf(Arc<UnixStream>);

impl Read for ReadHalf {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        (&*self.0).read(buf)
    }
}

/// Shut a client socket down (`shutdown(2)`, SHUT_RDWR) so a peer blocked in
/// `read()` gets EOF immediately. Declared directly (libc is already a
/// transitive dependency via rusqlite/crossterm; no Cargo.toml change).
fn libc_shutdown(stream: &UnixStream) -> std::io::Result<()> {
    // SAFETY: `shutdown(2)` on a live socket fd; `stream` outlives the call.
    // Idempotent — a second call just returns ENOTCONN, which we surface.
    let rc = unsafe { shutdown_syscall(std::os::unix::io::AsRawFd::as_raw_fd(stream)) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

unsafe fn shutdown_syscall(fd: std::os::raw::c_int) -> std::os::raw::c_int {
    unsafe extern "C" {
        fn shutdown(socket: std::os::raw::c_int, how: std::os::raw::c_int) -> std::os::raw::c_int;
    }
    unsafe { shutdown(fd, 2 /* SHUT_RDWR */) }
}

impl Daemon {
    /// Bind `$XDG_RUNTIME_DIR/mypi.sock` (or `path`) and run.
    ///
    /// A stale socket file from a dead daemon is removed; a socket held by a
    /// **live** daemon makes `bind` fail and that is the whole answer (SERVER
    /// §1: two daemons racing is not a designed-for case).
    pub fn bind(path: PathBuf, hub: SessionHub, spec: SessionSpec, idle_limit: std::time::Duration) -> anyhow::Result<Self> {
        let _ = std::fs::remove_file(&path); // stale only: bind below fails on a live one
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let listener = std::os::unix::net::UnixListener::bind(&path)
            .map_err(|e| anyhow::anyhow!("无法绑定 {}: {e}", path.display()))?;
        let (tx, cmd_rx) = std::sync::mpsc::channel::<Command>();
        let (reg_tx, reg_rx) = std::sync::mpsc::channel::<ConnectionHandle>();
        // Accept loop: one thread, forever. Each accepted connection gets a
        // reader thread (into `tx`) and a writer thread (from its queue tx),
        // plus its `ConnectionHandle` sent to the main loop for registration.
        let accept_tx = tx.clone();
        let accept = std::thread::Builder::new()
            .name("daemon-accept".into())
            .spawn(move || accept_loop(listener, accept_tx, reg_tx))?;
        Ok(Self {
            _accept: accept,
            hub,
            spec,
            conns: BTreeMap::new(),
            next_conn_id: 1,
            cmd_rx,
            cmd_tx: tx.clone(),
            reg_rx,
            watchers: HashMap::new(),
            shipped: HashMap::new(),
            last_stream: HashMap::new(),
            paging: HashSet::new(),
            idle_since: None,
            idle_limit,
            shutting_down: false,
        })
    }

    /// Run the daemon until idle-exit or `quit`. Blocks; returns when done.
    pub fn serve(mut self) -> anyhow::Result<()> {
        while !self.shutting_down {
            // Commands from the front ends, with a tick timeout so the hub is
            // drained even when nobody sends anything.
            match self.cmd_rx.recv_timeout(TICK) {
                Ok(Command::Msg { conn, msg }) => self.handle(conn, msg),
                Ok(Command::Page {
                    conn,
                    session,
                    newer,
                    blocks,
                }) => self.ship_page(conn, session, newer, blocks),
                Ok(Command::Gone { conn }) => self.drop_conn(conn),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    // Every connection thread is gone: nothing left to serve.
                    break;
                }
            }
            // Register connections the accept loop admitted meanwhile.
            while let Ok(handle) = self.reg_rx.try_recv() {
                let (outbox_tx, outbox_rx) = std::sync::mpsc::channel::<ServerMsg>();
                let id = self.next_conn_id;
                self.next_conn_id += 1;
                let conn_stream = handle.stream.clone();
                std::thread::spawn(move || write_loop(handle.stream, outbox_rx));
                self.conns.insert(
                    id,
                    Conn {
                        stream: conn_stream,
                        outbox: VecDeque::new(),
                        tx: outbox_tx,
                        attached: None,
                    },
                );
                handle.ready.send(id).ok();
            }
            self.pump();
            self.idle_check();
        }
        // Wake every reader thread (dropping the command channel closes their
        // loop's sender side → they see send-failure → they exit). The accept
        // thread is detached on drop of its handle; process exit follows.
        Ok(())
    }

    fn pump(&mut self) {
        // Drain every session's event channel and fan out the deltas.
        for (id, changes) in self.hub.drain_all() {
            let mut msgs = Vec::new();
            let mut tail_dirty = false;
            let mut stream_dirty = false;
            let mut state_dirty = false;
            for c in changes {
                match c {
                    Change::Transcript | Change::ToolActivity => tail_dirty = true,
                    Change::TurnDone => {
                        tail_dirty = true;
                        state_dirty = true;
                    }
                    // Consecutive Stream changes collapse into ONE snapshot:
                    // the message carries the whole in-flight half-sentence.
                    Change::Stream => stream_dirty = true,
                    Change::Session => {
                        tail_dirty = true;
                        state_dirty = true;
                    }
                    Change::None => {}
                }
            }
            if stream_dirty {
                let view = self
                    .hub
                    .get(id)
                    .map(|s| s.stream_view().clone())
                    .unwrap_or_default();
                if Self::stream_changed(self.last_stream.get(&id), &view) {
                    self.last_stream.insert(id, view.clone());
                    msgs.push(Self::stream_msg_of(view));
                }
            }
            if tail_dirty {
                // Tail delta, or a full snapshot when the generation jumped
                // (replace_transcript / resume / compaction). Per watcher:
                // a client that attached later still gets its own snapshot.
                let conns = self.watchers.get(&id).cloned().unwrap_or_default();
                for cid in conns {
                    for m in self.transcript_msgs_for(id, cid) {
                        self.enqueue(cid, m);
                    }
                    self.enqueue(cid, self.state_msg(id));
                }
                state_dirty = false;
            }
            if state_dirty {
                msgs.push(self.state_msg(id));
            }
            if msgs.is_empty() {
                continue;
            }
            self.fanout(id, Fanout { msgs });
        }
        // 转录尾巴再**对账**一遍：它是派生状态，可以在一个 `Change` 都不发
        // 的情况下长出来——用户自己那条消息就是反例，`Session::start_turn`
        // 把它直接写进转录，报的却是 `Change::Stream`，于是只按事件发尾巴的
        // 话它会一直压到回合结束：回车之后屏幕上什么都不动，最后和回复一起
        // 冒出来。账本已经前进的会话这里返回空，上面刚发过的不会重复；
        // 只补尾巴不补状态，因为前端看得见的顺序是「转录先于描述它的状态」。
        for id in self.watchers.keys().copied().collect::<Vec<_>>() {
            let conns = self.watchers.get(&id).cloned().unwrap_or_default();
            for cid in conns {
                for m in self.transcript_msgs_for(id, cid) {
                    self.enqueue(cid, m);
                }
            }
        }
        // Ship everything.
        let conn_ids: Vec<u64> = self.conns.keys().copied().collect();
        for cid in conn_ids {
            self.flush_outbox(cid);
        }
    }

    fn handle(&mut self, conn: u64, msg: ClientMsg) {
        match msg {
            ClientMsg::Hello { proto, client: _ } => {
                if proto != PROTO_VERSION {
                    self.send(
                        conn,
                        ServerMsg::Error {
                            code: ErrorCode::ProtoMismatch,
                            message: format!("daemon speaks proto {PROTO_VERSION}, client said {proto}"),
                        },
                    );
                    // Refuse the connection outright: a version mismatch is not
                    // recoverable in-band.
                    self.drop_conn(conn);
                } else {
                    self.send(conn, ServerMsg::HelloOk {
                        proto: PROTO_VERSION,
                        commands: crate::server::wire::command_table(),
                    });
                }
            }
            ClientMsg::Attach { id } => self.attach(conn, id),
            ClientMsg::NeedOlder { before, count } => {
                self.need_page(conn, before, count, false)
            }
            ClientMsg::NeedNewer { after, count } => self.need_page(conn, after, count, true),
            ClientMsg::Detach => {
                if let Some(c) = self.conns.get_mut(&conn)
                    && let Some(old) = c.attached.take()
                {
                    self.unwatch(conn, old);
                }
            }
            ClientMsg::DeleteSession { id } => {
                // Whoever was watching it goes first: their next submit would
                // otherwise resume a session whose rows are about to vanish.
                if let Some(list) = self.watchers.remove(&id) {
                    for c in list {
                        if let Some(conn) = self.conns.get_mut(&c)
                            && conn.attached == Some(id)
                        {
                            conn.attached = None;
                        }
                    }
                }
                // The front end re-requests the list itself after a successful
                // delete, so this answers only on failure (a silent success
                // would leave the picker showing a row that is gone).
                if let Err(e) = self.hub.delete_session(id) {
                    self.send(
                        conn,
                        ServerMsg::Error {
                            code: ErrorCode::Internal,
                            message: format!("删除会话 {id} 失败：{e:#}"),
                        },
                    );
                }
            }
            ClientMsg::ListSessions { under } => {
                // The hub owns the database; the query is read-only, cheap,
                // and single-shot (one statement for the whole list).
                let root = under.as_deref().map(std::path::Path::new);
                let sessions = match self.hub.list_session_rows(root) {
                    Ok(v) => v
                        .into_iter()
                        .map(|r| SessionInfo {
                            id: r.meta.id,
                            name: r.meta.name,
                            started_at: r.meta.started_at,
                            cwd: r.meta.cwd,
                            first_message: r.first_user,
                            bytes: r.bytes,
                        })
                        .collect(),
                    Err(e) => {
                        self.send(
                            conn,
                            ServerMsg::Error {
                                code: ErrorCode::Internal,
                                message: format!("list_sessions 失败：{e:#}"),
                            },
                        );
                        return;
                    }
                };
                self.send(conn, ServerMsg::Sessions { sessions });
            }
            ClientMsg::ListRounds { id } => {
                match self.hub.rounds(id) {
                    Ok(v) => {
                        let rounds = v
                            .into_iter()
                            .map(|r| RoundInfo {
                                seq: r.seq,
                                ts: r.ts,
                                model: r.model,
                                protocol: r.protocol,
                                base_url: r.base_url,
                                max_tokens: r.max_tokens,
                                stop_reason: r.stop_reason,
                                first_block: r.first_block,
                                last_block: r.last_block,
                            })
                            .collect();
                        self.send(conn, ServerMsg::Rounds { id, rounds });
                    }
                    Err(e) => self.send(
                        conn,
                        ServerMsg::Error {
                            code: ErrorCode::Internal,
                            message: format!("list_rounds 失败：{e:#}"),
                        },
                    ),
                }
            }
            ClientMsg::Replay { id, round } => {
                match self.hub.replay(id, round) {
                    Ok(rp) => {
                        self.send(conn, ServerMsg::Replay { id, replay: rp });
                    }
                    Err(e) => self.send(
                        conn,
                        ServerMsg::Error {
                            code: ErrorCode::NoSuchRound,
                            message: e,
                        },
                    ),
                }
            }
            ClientMsg::Submit { text } => self.submit(conn, text),
            ClientMsg::Command { name, args } => self.command(conn, &name, &args),
            ClientMsg::Interrupt => {
                if let Some(id) = self.conns.get(&conn).and_then(|c| c.attached) {
                    self.hub.interrupt(id);
                    crate::server::log::info("daemon", format!("interrupt session {id}"));
                }
                // Interrupting nothing is not an error: the front end may have
                // raced a just-finished round.
            }
            ClientMsg::Logs { limit } => {
                let records: Vec<LogRecord> = crate::server::log::recent(limit)
                    .into_iter()
                    .map(|r| LogRecord {
                        at_ms: r.at_ms as u64,
                        level: r.level.as_str().to_string(),
                        scope: r.scope,
                        text: r.text,
                    })
                    .collect();
                self.send(conn, ServerMsg::Logs { records });
            }
            ClientMsg::Quit => {
                self.send(
                    conn,
                    ServerMsg::HelloOk {
                        proto: PROTO_VERSION,
                        commands: crate::server::wire::command_table(),
                    },
                );
                self.flush_outbox(conn);
                self.shutting_down = true;
            }
        }
    }

    /// 前端点名要一段历史（`ClientMsg::{NeedOlder,NeedNewer}`）。
    ///
    /// 边界由前端给（`before` / `after` 是块 id），所以这里没有游标可以做主：
    /// 同一会话几个前端各看各的窗口，谁要的那段就回给谁。同一条连接同一方向
    /// 只放一段在飞，重复的开口直接丢掉（前端下一帧还会再要一次，见
    /// [`Self::ship_page`] 的失败处理）。
    fn need_page(&mut self, conn: u64, edge: i64, count: usize, newer: bool) {
        let Some(id) = self.conns.get(&conn).and_then(|c| c.attached) else {
            return;
        };
        let count = count.clamp(1, NEW_BLOCKS_MAX);
        if self.paging.insert((id, conn, newer)) {
            spawn_page(
                self.hub.db().to_path_buf(),
                conn,
                id,
                edge,
                count,
                newer,
                self.cmd_tx.clone(),
            );
        }
        // 前端在等这一段画出来（滚轮按着呢），立刻发，别等下一次 pump。
        self.flush_outbox(conn);
    }

    /// 按需历史：读到的那一段回给开口的连接。
    ///
    /// 空的一段**照发**（`blocks: []`）——那是"这个方向到头了"的终止符，前端
    /// 据此闩死，否则每滚一格都会白问一次。读失败则不回（`None`），闸门照开：
    /// 回空的会把前端骗进"到头了"的死角。
    fn ship_page(
        &mut self,
        conn: u64,
        session: i64,
        newer: bool,
        blocks: Option<Vec<crate::server::store::Block>>,
    ) {
        self.paging.remove(&(session, conn, newer));
        let Some(blocks) = blocks else {
            return;
        };
        let blocks: Vec<crate::server::wire::WireBlock> =
            blocks.into_iter().map(wire_block).collect();
        let msg = if newer {
            ServerMsg::NewerBlocks { blocks }
        } else {
            ServerMsg::OlderBlocks { blocks }
        };
        self.send(conn, msg);
        self.flush_outbox(conn);
    }

    fn attach(&mut self, conn: u64, id: i64) {
        // Resume if not already open. Errors (bad id, bad db) go back as
        // protocol errors; the connection stays.
        if self.hub.get(id).is_none()
            && let Err(e) = self.hub.resume(id, self.spec.clone())
        {
            self.send(
                conn,
                ServerMsg::Error {
                    code: ErrorCode::NoSuchSession,
                    message: format!("无法附着会话 {id}：{e:#}"),
                },
            );
            return;
        }
        if let Some(c) = self.conns.get_mut(&conn) {
            // One watched session per connection: a re-attach detaches first.
            if let Some(old) = c.attached.replace(id) {
                self.unwatch(conn, old);
            }
        }
        self.watchers.entry(id).or_default().push(conn);
        // Snapshot: everything a fresh renderer needs, in order.
        self.send(conn, ServerMsg::Attached { session_id: id });
        for m in self.transcript_msgs_for(id, conn) {
            self.send(conn, m);
        }
        self.send(conn, self.stream_msg(id));
        self.send(conn, self.state_msg(id));
        // Attach responses must ship immediately (a front end blocks on them
        // before drawing); the periodic pump would also get there, but with
        // unbounded latency.
        self.flush_outbox(conn);
    }

    /// Run a slash command on the attached session.
    ///
    /// The front end did the *recognition* (it has the table); the meaning lives
    /// here, because every one of these is a session operation and the session is
    /// the only writer of the transcript.
    fn command(&mut self, conn: u64, name: &str, args: &str) {
        // A command needs a session, and typing one in the draft state is as
        // clear a statement of intent as typing a message: create it, then run.
        let Some(id) = self.ensure_session(conn) else {
            return;
        };
        let outcome = match crate::server::commands::lookup(name) {
            Some(spec) => {
                let session = self.hub.get_mut(id);
                match session {
                    Some(s) => s.run_command(spec, args),
                    None => Err(anyhow::anyhow!("会话 {id} 不在内存里")),
                }
            }
            None => Err(anyhow::anyhow!("未知命令 {name}")),
        };
        if let Err(e) = outcome {
            // A refused command is narrated, not raised: the wire stays up and
            // the user sees why nothing happened.
            if let Some(s) = self.hub.get_mut(id) {
                s.notice(&format!("{name} 没做成：{e:#}"));
            }
        }
        // Whatever happened, the transcript (and thus the front end) is refreshed
        // by the pump on the next tick.
        self.pump();
    }

    /// The session this connection is attached to, creating it on first use.
    ///
    /// Draft state has no session row yet; the first thing that *needs* one —
    /// a submit, or a command like `/name` — creates it. (SERVER.md §1: there
    /// is no `new_session`; the first use IS the session creation.)
    fn ensure_session(&mut self, conn: u64) -> Option<i64> {
        if let Some(id) = self.conns.get(&conn).and_then(|c| c.attached) {
            return Some(id);
        }
        match self.hub.open_new(self.spec.clone()) {
            Ok(id) => {
                if let Some(c) = self.conns.get_mut(&conn) {
                    c.attached = Some(id);
                }
                self.watchers.entry(id).or_default().push(conn);
                self.send(conn, ServerMsg::Attached { session_id: id });
                Some(id)
            }
            Err(e) => {
                self.send(
                    conn,
                    ServerMsg::Error {
                        code: ErrorCode::Internal,
                        message: format!("无法创建会话：{e:#}"),
                    },
                );
                None
            }
        }
    }

    fn submit(&mut self, conn: u64, text: String) {
        if let Some(id) = self.ensure_session(conn) {
            self.do_submit(conn, id, text);
        }
    }

    fn do_submit(&mut self, conn: u64, id: i64, text: String) {
        if self.hub.submit(id, &text) {
            // The user's own message reaches every watcher through the
            // transcript tail; pump() runs right after this command.
            return;
        }
        self.send(
            conn,
            ServerMsg::Error {
                code: ErrorCode::Busy,
                message: format!("会话 {id} 正在回复中"),
            },
        );
    }

    /// The transcript messages for ONE watcher of `id` (empty = nothing new).
    ///
    /// The ledger is per (session, connection): two front ends attaching at
    /// different times need different snapshots, and a shared ledger would
    /// ship conn2 nothing because conn1 already took the tail.
    ///
    /// **Two ledgers, because a front end holds two things.** Its window is
    /// blocks (with ids) and its live tail is entries that are not rows yet.
    /// A delta therefore has two possible halves:
    ///
    /// - **live grew** → ship the new entries (`entry` / `entry_many`). This
    ///   is the streaming hot path and costs exactly what it used to.
    /// - **a round landed** → ship the newly stored blocks (`blocks`) plus the
    ///   live list as it now stands. The live half is a *replacement*: that is
    ///   what makes the hand-off exact — the entries that just became rows
    ///   have to leave the front end's live tail, and only the server knows
    ///   which ones those are (`SessionState::note_persisted`).
    ///
    /// Nothing is chunked: a snapshot is the branch **tail** ([`HISTORY_TAIL`]
    /// blocks ≈ 1 MB), not a whole conversation, so it fits one line.
    fn transcript_msgs_for(&mut self, id: i64, conn: u64) -> Vec<ServerMsg> {
        let (generation, live) = {
            let Some(session) = self.hub.get(id) else {
                return Vec::new();
            };
            (
                session.state.transcript_generation(),
                session.live().to_vec(),
            )
        };
        let key = (id, conn);
        if let Some(s) = self.shipped.get(&key).copied()
            && s.generation == generation
        {
            let mut msgs = Vec::new();
            if live.len() > s.live_len {
                let tail = &live[s.live_len..];
                msgs.push(if tail.len() == 1 {
                    ServerMsg::Entry {
                        entry: tail[0].clone(),
                    }
                } else {
                    ServerMsg::EntryMany {
                        entries: tail.to_vec(),
                    }
                });
            }
            let fresh = self.stored_between(id, s.block_id, NEW_BLOCKS_MAX);
            if !fresh.is_empty() || live.len() < s.live_len {
                // A shrunken live list with no new blocks cannot happen (a
                // persist writes at least one block), but a front end that
                // misses the replacement would keep showing entries that are
                // already blocks — so resync it either way.
                let newest = fresh.last().map(|b| b.id).unwrap_or(s.block_id);
                msgs.push(ServerMsg::Blocks {
                    blocks: fresh,
                    live: live.clone(),
                });
                self.shipped.insert(
                    key,
                    Ship {
                        live_len: live.len(),
                        block_id: newest,
                        generation,
                    },
                );
            } else if live.len() != s.live_len {
                self.shipped.insert(
                    key,
                    Ship {
                        live_len: live.len(),
                        generation,
                        ..s
                    },
                );
            }
            return msgs;
        }
        // First ship for this watcher, or the transcript was replaced
        // wholesale: the window starts at the branch tail, plus whatever has
        // not been stored yet.
        let blocks = self.tail_blocks(id, crate::server::hub::SessionHub::HISTORY_TAIL);
        let newest = blocks.last().map(|b| b.id).unwrap_or(0);
        let msg = ServerMsg::Transcript {
            blocks,
            live: live.clone(),
        };
        self.shipped.insert(
            key,
            Ship {
                generation,
                live_len: live.len(),
                block_id: newest,
            },
        );
        vec![msg]
    }

    /// 会话的尾巴那截块（窗口的起点）。
    fn tail_blocks(&self, id: i64, limit: usize) -> Vec<crate::server::wire::WireBlock> {
        let Some(sid) = self.hub.get(id).and_then(|s| s.state.session_id()) else {
            return Vec::new();
        };
        let Some(st) = self.hub.get(id).and_then(|s| s.state.store()) else {
            return Vec::new();
        };
        st.load_tail(sid, limit)
            .map(|(blocks, _)| blocks.into_iter().map(wire_block).collect())
            .unwrap_or_default()
    }

    /// 库里比 `after` 更新的块（有新回合落盘时补发；空 = 没有）。
    fn stored_between(
        &self,
        id: i64,
        after: i64,
        limit: usize,
    ) -> Vec<crate::server::wire::WireBlock> {
        let Some(session) = self.hub.get(id) else {
            return Vec::new();
        };
        let (Some(sid), Some(st)) = (session.state.session_id(), session.state.store()) else {
            return Vec::new();
        };
        st.load_after(sid, after, limit)
            .map(|blocks| blocks.into_iter().map(wire_block).collect())
            .unwrap_or_default()
    }


    /// 这一帧流式快照值不值得发：和上一帧**逐字段**相同就不发。
    ///
    /// 不比长度：工具输出的尾巴是滑动窗口，长度可以一模一样而内容是新的。
    fn stream_changed(last: Option<&StreamView>, now: &StreamView) -> bool {
        last != Some(now)
    }

    fn stream_msg(&self, id: i64) -> ServerMsg {
        let v = self
            .hub
            .get(id)
            .map(|s| s.stream_view().clone())
            .unwrap_or_default();
        Self::stream_msg_of(v)
    }

    /// Build the wire message from an already-snapshotted view (the pump
    /// compares it against the last one it sent; attach always sends one).
    fn stream_msg_of(v: StreamView) -> ServerMsg {
        let run_state = v.run_state();
        ServerMsg::Stream {
            active: v.active,
            text: v.text,
            reasoning: v.reasoning,
            reasoning_done: v.reasoning_done,
            live: v.live,
            run_state,
            tool_output: v.tool_output,
        }
    }

    fn state_msg(&self, id: i64) -> ServerMsg {
        let Some(s) = self.hub.get(id) else {
            return ServerMsg::State {
                model: String::new(),
                name: None,
                cwd: String::new(),
                spend_usd: 0.0,
                last_prompt_tokens: 0,
                context_window: 0,
                busy: false,
            };
        };
        let (spend_usd, last_prompt_tokens) = s.spend();
        ServerMsg::State {
            model: s.model_name().to_string(),
            name: s.session_name().map(str::to_string),
            cwd: s.cwd().to_string_lossy().into_owned(),
            spend_usd,
            last_prompt_tokens,
            context_window: s.context_window(),
            busy: s.busy(),
        }
    }

    fn fanout(&mut self, id: i64, f: Fanout) {
        let Some(watchers) = self.watchers.get(&id).cloned() else {
            return;
        };
        for cid in watchers {
            for m in &f.msgs {
                self.enqueue(cid, m.clone());
            }
        }
    }

    fn enqueue(&mut self, conn: u64, msg: ServerMsg) {
        let Some(c) = self.conns.get_mut(&conn) else {
            return;
        };
        if c.outbox.len() > OUTBOX_CAP {
            // The front end stopped reading facts and will not catch up; call
            // it dead. Disconnecting triggers the no-watcher stop (§4).
            self.drop_conn(conn);
            return;
        }
        c.outbox.push_back(msg);
    }

    fn send(&mut self, conn: u64, msg: ServerMsg) {
        self.enqueue(conn, msg);
        // Ship immediately: a refusal (`proto_mismatch`) queued right before
        // `drop_conn` must reach the peer while the connection still exists.
        self.flush_outbox(conn);
    }

    /// Move everything queued for one connection onto its write thread.
    fn flush_outbox(&mut self, conn: u64) {
        let Some(c) = self.conns.get_mut(&conn) else { return };
        while let Some(msg) = c.outbox.pop_front() {
            if c.tx.send(msg).is_err() {
                break; // write thread gone; reader will report Gone
            }
        }
    }

    fn unwatch(&mut self, conn: u64, id: i64) {
        if let Some(list) = self.watchers.get_mut(&id) {
            list.retain(|c| *c != conn);
            if list.is_empty() {
                self.watchers.remove(&id);
                // The last watcher left. A running round stops now: nobody is
                // watching, so no reason to keep generating (SERVER.md §4).
                self.hub.interrupt(id);
            }
        }
    }

    /// Drop a connection: flush whatever is queued, then close the socket.
    ///
    /// Flushing matters: a refusal (`proto_mismatch`) or a final state frame
    /// queued right before the drop must still reach the peer — the write
    /// thread drains the channel (which `drop_conn` closes by dropping every
    /// `tx` clone) *after* sending the backlog, then shuts the socket down so
    /// the client's blocking read ends promptly.
    fn drop_conn(&mut self, conn: u64) {
        if let Some(c) = self.conns.remove(&conn) {
            if let Some(id) = c.attached {
                self.unwatch(conn, id);
            }
            drop(c.tx); // closes the write loop's rx after the backlog
            let stream = c.stream;
            std::thread::spawn(move || {
                // Give the write thread a beat to drain the backlog, then cut
                // the peer loose. A client blocked in read() must not hang.
                std::thread::sleep(std::time::Duration::from_millis(150));
                let _ = libc_shutdown(&stream);
            });
        }
    }

    fn idle_check(&mut self) {
        let ids = self.hub.ids();
        // 会话关掉之后，两张按会话记账的表留着条目只是长胖：`shipped` 还按
        // (会话, 连接) 记，连接号只增不减。
        self.shipped.retain(|(id, _), _| ids.contains(id));
        self.last_stream.retain(|id, _| ids.contains(id));
        self.paging.retain(|(id, _, _)| ids.contains(id));
        let any_busy = ids.iter().any(|id| {
            self.hub
                .get(*id)
                .map(|s| s.busy())
                .unwrap_or(false)
        });
        let any_watcher = !self.watchers.is_empty();
        if any_busy || any_watcher || self.shutting_down {
            self.idle_since = None;
        } else if self.idle_since.is_none() {
            self.idle_since = Some(std::time::Instant::now());
        }
        if let Some(t) = self.idle_since
            && t.elapsed() >= self.idle_limit
        {
            self.shutting_down = true;
        }
    }
}

#[cfg(test)]
mod tests;
