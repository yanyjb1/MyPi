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
    /// 上一次**发出去**的状态帧（按内容比对，不是按事件）。
    ///
    /// 状态帧的触发不能只挂在 `Change::{TurnDone,Session}` 上：命令
    /// （`/switch`、`/name`、`/cdp`）直接改会话字段、不产生 `SessionEvent`，
    /// 提交那一刻也只报 `Change::Stream`。少了这本账，"切了模型要等这一轮
    /// 跑完才改"和"spinner 死活不转"就是必然。
    last_state: HashMap<i64, ServerMsg>,
    /// (会话, 连接, 方向) → 后台正在读那一段。前端连着滚会重复开口（手不会
    /// 只滚一格），没有这道闸门同一个区间会读两遍、前端收到重复内容。
    paging: HashSet<(i64, u64, bool)>,
    /// Idle-exit bookkeeping (SERVER.md §4): nothing attached, nothing busy.
    idle_since: Option<std::time::Instant>,
    idle_limit: std::time::Duration,
    /// 上一跳有没有会话在跑回合。回合刚结束的那一跳仍然要对账一次：那一回
    /// 合写的条目是在 `busy()` 为真时落的，`busy()` 变假并不代表账已经结清。
    was_busy: bool,
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
    /// A connection was just accepted and handed to `reg_rx`.
    ///
    /// `reg_rx` is a separate channel from `cmd_rx`, and `mpsc` cannot be
    /// selected on, so the main loop would otherwise only drain it on the next
    /// tick. That cost the attach path a full `TICK` before the first frame
    /// could be painted (measured: 36ms → 51ms at TICK=20ms). This variant is
    /// a pure wake-up: it carries no payload and intentionally does nothing
    /// when handled, it just makes `recv_timeout` return so registration runs
    /// now instead of up to a tick later.
    Woke,
}

/// How long the daemon waits between ticks **with no command arriving**.
///
/// This is the idle poll, not the latency knob: `recv_timeout` returns the
/// moment a command (submit, page, attach) lands, so interactive latency does
/// not depend on it. What it does decide is how often the loop wakes to ask
/// "anything new?" for a turn running in another thread. At 5ms that was 200
/// wakeups a second for the whole life of every session — measured at 0.5% of
/// a core with one client attached and nothing happening, against ~0.1% at
/// 20ms. 20ms is still far below the ~50ms at which a person notices a
/// streamed word appearing, and the reconciliation gate in `pump` keeps the
/// per-tick SQL out of the idle path entirely.
///
/// One thing this *cannot* cover is the accept path: a new connection is
/// handed over on `reg_rx`, a different channel, which `mpsc` cannot select
/// on. The accept loop therefore sends `Command::Woke` so the wait ends
/// immediately — without that, raising this value from 5ms to 20ms added 15ms
/// to the attach cold start (measured 36ms → 51ms).
const TICK: std::time::Duration = std::time::Duration::from_millis(20);

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
        // Wake the main loop now: `reg_rx` is drained after `recv_timeout`
        // returns, and nothing else guarantees that happens before the next
        // tick. Without this the first frame of an attach waits a full TICK.
        if cmd_tx.send(Command::Woke).is_err() {
            break;
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
        let t0 = std::time::Instant::now();
        let store = Store::open(&db);
        let t_open = t0.elapsed();
        let t1 = std::time::Instant::now();
        let blocks = store.ok().and_then(|st| {
            if newer {
                st.load_after(session, edge, count).ok()
            } else {
                st.load_before(session, edge, count).ok().map(|(b, _)| b)
            }
        });
        let t_read = t1.elapsed();
        let n = blocks.as_ref().map(|b| b.len()).unwrap_or(0);
        eprintln!(
            "[page] open={:.2}ms read={:.2}ms n={} dir={}",
            t_open.as_secs_f64() * 1e3,
            t_read.as_secs_f64() * 1e3,
            n,
            if newer { "new" } else { "old" }
        );
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
            .map_err(|e| anyhow::anyhow!("cannot bind {}: {e}", path.display()))?;
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
            last_state: HashMap::new(),
            paging: HashSet::new(),
            idle_since: None,
            idle_limit,
            was_busy: false,
            shutting_down: false,
        })
    }

    /// Run the daemon until idle-exit or `quit`. Blocks; returns when done.
    pub fn serve(mut self) -> anyhow::Result<()> {
        while !self.shutting_down {
            // Commands from the front ends, with a tick timeout so the hub is
            // drained even when nobody sends anything.
            let acted = match self.cmd_rx.recv_timeout(TICK) {
                Ok(Command::Msg { conn, msg }) => {
                    self.handle(conn, msg);
                    true
                }
                Ok(Command::Page {
                    conn,
                    session,
                    newer,
                    blocks,
                }) => {
                    self.ship_page(conn, session, newer, blocks);
                    true
                }
                Ok(Command::Gone { conn }) => {
                    self.drop_conn(conn);
                    true
                }
                // Pure wake-up: the work is draining `reg_rx` just below.
                Ok(Command::Woke) => false,
                Err(RecvTimeoutError::Timeout) => false,
                Err(RecvTimeoutError::Disconnected) => {
                    // Every connection thread is gone: nothing left to serve.
                    break;
                }
            };
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
            self.pump(acted);
            self.idle_check();
        }
        // Wake every reader thread (dropping the command channel closes their
        // loop's sender side → they see send-failure → they exit). The accept
        // thread is detached on drop of its handle; process exit follows.
        Ok(())
    }

    /// `acted` = a command ran this iteration.
    ///
    /// The reconciliation pass at the end has a different trigger from the
    /// delta pass, which is why one flag does not cover both:
    ///
    /// - the **delta** pass needs a drained change (that is what a change is);
    /// - the **reconciliation** pass needs "something may have landed since it
    ///   last looked". Entries reach storage without a tail-shaped change:
    ///   `Session::start_turn` writes the user's own message while reporting
    ///   `Change::Stream`, and a tool round's blocks are persisted as the round
    ///   proceeds. Running it on *every* tick is what caught those — and also
    ///   what made an attached-but-idle daemon issue a per-watcher SQL query
    ///   200 times a second (measured: 0.5% of a core, forever).
    ///
    /// So it runs while a turn is in flight (the only window in which blocks
    /// land unannounced), on the tick a command ran, and on the **one** tick
    /// after the last busy one — the round just ended, and the entries it wrote
    /// while `busy()` was still true are exactly what that pass owes the
    /// watchers. Every other tick skips it.
    fn pump(&mut self, acted: bool) {
        // Drain every session's event channel and fan out the deltas.
        for (id, changes) in self.hub.drain_all() {
            let mut msgs = Vec::new();
            let mut tail_dirty = false;
            let mut stream_dirty = false;
            let mut state_dirty = false;
            for c in changes {
                match c {
                    Change::Transcript | Change::ToolActivity => {
                        tail_dirty = true;
                        // 条目落册的同时流式槽位也可能被改：`ToolStart` 会清空
                        // 正文缓冲（那半句已经随工具条目存档），`ToolFinish` 会
                        // 丢掉工具输出尾巴——两者报的都是条目变更。只认
                        // `Change::Stream` 的话，前端会一直挂着那半句，屏幕上
                        // 正文区和工具卡片各一份。
                        stream_dirty = true;
                    }
                    Change::TurnDone => {
                        tail_dirty = true;
                        state_dirty = true;
                        // 收尾会把半句取走（`finalize_round` 的 `mem::take`），
                        // 前端也得知道它没了。
                        stream_dirty = true;
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
            if stream_dirty && let Some(m) = self.fresh_stream(id) {
                msgs.push(m);
            }
            if tail_dirty {
                // Tail delta, or a full snapshot when the generation jumped
                // (replace_transcript / resume / compaction). Per watcher:
                // a client that attached later still gets its own snapshot.
                let conns = self.watchers.get(&id).cloned().unwrap_or_default();
                // 状态按会话记账，但每个 watcher 都得收到同一份。
                let state = self.fresh_state(id);
                for cid in conns {
                    for m in self.transcript_msgs_for(id, cid) {
                        self.enqueue(cid, m);
                    }
                    if let Some(m) = state.clone() {
                        self.enqueue(cid, m);
                    }
                }
                state_dirty = false;
            }
            if state_dirty && let Some(m) = self.fresh_state(id) {
                msgs.push(m);
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
        //
        // 跑不跑的判据见 `pump` 的文档：有回合在飞（块可能悄悄落地）、刚有人
        // 下过命令、或者**回合刚结束的那一跳**（那时 `busy()` 已是 false，而
        // 这一回合写的条目正是欠 watcher 的账）。
        let busy = self
            .hub
            .ids()
            .iter()
            .any(|id| self.hub.get(*id).map(|s| s.busy()).unwrap_or(false));
        let settling = !busy && self.was_busy;
        self.was_busy = busy;
        if !(acted || busy || settling) {
            self.flush_all();
            return;
        }
        for id in self.watchers.keys().copied().collect::<Vec<_>>() {
            let conns = self.watchers.get(&id).cloned().unwrap_or_default();
            // 状态帧的主场：命令改完模型/名字，或者回合刚收尾，都在这里把
            // 新的状态发出去（转录排在它前面，前端看到的顺序才是对的）。
            let state = self.fresh_state(id);
            for cid in conns {
                for m in self.transcript_msgs_for(id, cid) {
                    self.enqueue(cid, m);
                }
                if let Some(m) = state.clone() {
                    self.enqueue(cid, m);
                }
            }
        }
        // Ship everything.
        self.flush_all();
    }

    /// Write every connection's queued messages out.
    fn flush_all(&mut self) {
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
                            message: format!("failed to delete session {id}: {e:#}"),
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
                                message: format!("list_sessions failed: {e:#}"),
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
                            message: format!("list_rounds failed: {e:#}"),
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
            ClientMsg::Rewind { session, block } => {
                // 前端点名目标；连接必须正附着在它上面。不满足要**说**：
                // 静默 return 的话前端在树页上回车，永远等不到快照，回到
                // 主区看到的是旧转录——"回退"看起来成功了，其实什么都没发生。
                if self.conns.get(&conn).and_then(|c| c.attached) != Some(session) {
                    self.send(
                        conn,
                        ServerMsg::Error {
                            code: ErrorCode::NoSuchSession,
                            message: format!(
                                "rewind ignored: connection is not attached to session {session}"
                            ),
                        },
                    );
                    return;
                }
                // 选中"用户消息"节点 = 把那句话收回输入框重发。内容要在
                // **剪枝前**读（剪枝模式它自己也会被删）；非用户节点没有可
                // 收回的东西，prefill 为 None。
                let prefill = Store::open(self.hub.db())
                    .ok()
                    .and_then(|st| st.user_text_of(block).ok())
                    .flatten();
                match self.hub.rewind_to(session, block) {
                    Ok(new_id) => {
                        // 剪模式 new_id == session；fork 模式换了行，观察者
                        // 跟着搬（同 `submit` 的 fork 分支）。
                        if new_id != session {
                            self.unwatch(conn, session);
                        }
                        if let Some(c) = self.conns.get_mut(&conn) {
                            c.attached = Some(new_id);
                        }
                        self.watch(conn, new_id);
                        // attach 规格的快照，立刻发：前端此刻站在旧树页上等
                        // 回包，回主区第一帧就要画新前缀。
                        self.send(conn, ServerMsg::Attached { session_id: new_id });
                        if let Some(text) = prefill {
                            self.send(conn, ServerMsg::Prefill { text });
                        }
                        for m in self.transcript_msgs_for(new_id, conn) {
                            self.send(conn, m);
                        }
                        self.send(conn, self.stream_msg(new_id));
                        if let Some(s) = self.hub.get(new_id) {
                            self.last_stream.insert(new_id, s.stream_view().clone());
                        }
                        let state = self.state_msg(new_id);
                        self.last_state.insert(new_id, state.clone());
                        self.send(conn, state);
                        self.flush_outbox(conn);
                    }
                    Err(e) => {
                        self.send(
                            conn,
                            ServerMsg::Error {
                                code: ErrorCode::Internal,
                                message: format!("failed to rewind to block {block}: {e:#}"),
                            },
                        );
                    }
                }
            }
            ClientMsg::Interrupt { session } => {
                // The front end names its target; the connection must be the
                // one attached to it. A stale id (raced a round that already
                // ended, or a session the connection never owned) is not an
                // error — the front end may have raced a just-finished round.
                if self.conns.get(&conn).and_then(|c| c.attached) == Some(session) {
                    self.hub.interrupt(session);
                    crate::server::log::info("daemon", format!("interrupt session {session}"));
                    // fire-and-forget：flag 已设就是服务端能给的全部保证。前端的
                    // 可输入态由它自己的乐观反馈（发打断的那一跳已切回）与随后
                    // 的 `State{busy:false}`（权威收尾）共同收敛，无需专门回执。
                    self.flush_outbox(conn);
                }
            }
            ClientMsg::Tree { session } => {
                let att = self.conns.get(&conn).and_then(|c| c.attached);
                let id = session.or(att);

                let Some(id) = id else {
                    // 没有会话可看（草稿态）：回一个空树，前端画"（空）"。
                    self.send(
                        conn,
                        ServerMsg::Tree {
                            nodes: Vec::new(),
                            forks: Vec::new(),
                            current_block: 0,
                        },
                    );
                    return;
                };
                match self.hub.tree(id) {
                    Ok((nodes, forks, current_block)) => {
                        self.send(conn, ServerMsg::Tree { nodes, forks, current_block });
                    }
                    Err(e) => self.send(
                        conn,
                        ServerMsg::Error {
                            code: ErrorCode::Internal,
                            message: format!("failed to read the session tree: {e}"),
                        },
                    ),
                }
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
        eprintln!("[page] ship n={} dir={}", blocks.len(), if newer { "new" } else { "old" });
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
                    message: format!("cannot attach to session {id}: {e:#}"),
                },
            );
            return;
        }
        if let Some(c) = self.conns.get_mut(&conn) {
            // One watched session per connection: a re-attach detaches first.
            if let Some(old) = c.attached.replace(id) {
                // re-attach 到同一个 id 时这一步不该摘：摘了就是"唯一观察者
                // 走了"，`unwatch` 会打断这个会话自己。
                if old != id {
                    self.unwatch(conn, old);
                }
            }
        }
        self.watch(conn, id);
        // Snapshot: everything a fresh renderer needs, in order.
        self.send(conn, ServerMsg::Attached { session_id: id });
        for m in self.transcript_msgs_for(id, conn) {
            self.send(conn, m);
        }
        self.send(conn, self.stream_msg(id));
        // 挂载是无条件对齐：这两份就是"上一次发出去的"，记账跟上，免得下一
        // 跳又原样重发一遍。
        if let Some(s) = self.hub.get(id) {
            self.last_stream.insert(id, s.stream_view().clone());
        }
        let state = self.state_msg(id);
        self.last_state.insert(id, state.clone());
        self.send(conn, state);
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
                    None => Err(anyhow::anyhow!("session {id} is not in memory")),
                }
            }
            None => Err(anyhow::anyhow!("unknown command {name}")),
        };
        if let Err(e) = outcome {
            // A refused command is narrated, not raised: the wire stays up and
            // the user sees why nothing happened.
            if let Some(s) = self.hub.get_mut(id) {
                s.notice(&format!("{name} did not go through: {e:#}"));
            }
        }
        // `/usage` 的账单是**给敲命令的这个前端**的一张面板，不是转录条目：
        // 别的 watcher 不该被弹一脸，所以它单独走一条消息，只回发起者。
        let panel = self.hub.get_mut(id).and_then(|s| s.take_usage_panel());
        if let Some((lang, groups)) = panel {
            self.send(conn, ServerMsg::Usage { lang, groups });
        }
        // Whatever happened, the transcript (and thus the front end) is refreshed
        // by the pump on the next tick.
        self.pump(true);
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
                        message: format!("cannot create the session: {e:#}"),
                    },
                );
                None
            }
        }
    }

    fn submit(&mut self, conn: u64, text: String) {
        if let Some(id) = self.ensure_session(conn) {
            self.watch(conn, id);
            self.do_submit(conn, id, text);
        }
    }

    fn do_submit(&mut self, conn: u64, id: i64, text: String) {
        // 模型不可用的会话不许开工：不静默拿默认模型顶上（上下文一长，账单是
        // 用户的），也不假装收到了。
        let blocked = self
            .hub
            .get(id)
            .and_then(|s| s.model_block())
            .map(str::to_string);
        if let Some(block) = blocked {
            self.send(
                conn,
                ServerMsg::Error {
                    code: ErrorCode::Internal,
                    message: block,
                },
            );
            return;
        }
        if self.hub.submit(id, &text) {
            // The user's own message reaches every watcher through the
            // transcript tail; pump() runs right after this command.
            return;
        }
        self.send(
            conn,
            ServerMsg::Error {
                code: ErrorCode::Busy,
                message: format!("session {id} is in the middle of a reply"),
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

    /// 流式帧的内容比对（和 [`Self::fresh_state`] 一个思路）。
    ///
    /// `Change::Stream` 不是唯一会让流式槽位变样的事件——`ToolStart` 清空正文
    /// 缓冲、`finalize_round` 取走半句，报的都是别的 `Change`。比对**内容**
    /// 而不是信任事件标签，才不会漏帧：漏一帧的后果是前端把已经存档的那半句
    /// 继续挂在正文区，和工具卡片上的同一段字重成两份。
    fn fresh_stream(&mut self, id: i64) -> Option<ServerMsg> {
        let fresh = {
            let Some(s) = self.hub.get(id) else {
                return None;
            };
            let view = s.stream_view();
            if self.last_stream.get(&id) == Some(view) {
                return None;
            }
            view.clone()
        };
        self.last_stream.insert(id, fresh.clone());
        Some(Self::stream_msg_of(fresh))
    }

    /// 状态帧的边沿检测：与上一次**发出去**的逐字段比对，变了才发。
    ///
    /// 这是 `/switch`、`/name`、`/cdp` 这类命令唯一的出路——它们直接改会话
    /// 字段，不产生任何 `SessionEvent`，delta pass 看不见它们；也是 spinner
    /// 亮起来的唯一出路——提交那一刻只报 `Change::Stream`，`busy()` 由假
    /// 变真没有任何事件。副作用是好的：状态帧从此只在真的变了的时候走线。
    fn fresh_state(&mut self, id: i64) -> Option<ServerMsg> {
        let now = self.state_msg(id);
        if self.last_state.get(&id) == Some(&now) {
            return None;
        }
        self.last_state.insert(id, now.clone());
        Some(now)
    }

    fn state_msg(&self, id: i64) -> ServerMsg {
        let Some(s) = self.hub.get(id) else {
            return ServerMsg::State {
                model: String::new(),
                name: None,
                cwd: String::new(),
                spend_usd: 0.0,
                currency: String::new(),
                last_prompt_tokens: 0,
                context_window: 0,
                ctx_estimate: 0,
                busy: false,
            };
        };
        let (spend_usd, last_prompt_tokens) = s.spend();
        ServerMsg::State {
            model: s.model_name().to_string(),
            name: s.session_name().map(str::to_string),
            cwd: s.cwd().to_string_lossy().into_owned(),
            spend_usd,
            // 货币符号跟着模型走：金额没有符号就是个无法解释的数。
            currency: s.currency_symbol().to_string(),
            last_prompt_tokens,
            context_window: s.context_window(),
            ctx_estimate: s.context_estimate(),
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

    /// 记下"这条连接在看这个会话"。**同一条连接同一个会话只记一份。**
    ///
    /// 去重不是优化，是正确性：`unwatch` 用 `retain` 一次摘掉**所有**等于
    /// `conn` 的项，所以重复记账会让"一次 unwatch"把整张列表清空，把仍然
    /// 盯着终端的用户误判成"没人看了"，当场打断正在跑的回合。
    fn watch(&mut self, conn: u64, id: i64) {
        let list = self.watchers.entry(id).or_default();
        if !list.contains(&conn) {
            list.push(conn);
        }
    }

    /// 摘掉一条连接对某个会话的观察。**列表真的空了就回收**：没有人在看，
    /// 就没有理由继续生成（SERVER.md §4）。
    ///
    /// 这是**所有权回收**，不是用户打断：不落 `stop_reason`、不回执——
    /// 走 `drop_turn` 的路子把在跑的回合安静停掉。用户按 Esc 走
    /// `ClientMsg::Interrupt` → [`SessionHub::interrupt`]。
    ///
    /// 前提是记账没出错：见 [`Self::watch`] 的去重。
    fn unwatch(&mut self, conn: u64, id: i64) {
        if let Some(list) = self.watchers.get_mut(&id) {
            list.retain(|c| *c != conn);
            if list.is_empty() {
                self.watchers.remove(&id);
                self.hub.drop_turn_of(id);
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
        self.last_state.retain(|id, _| ids.contains(id));
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
