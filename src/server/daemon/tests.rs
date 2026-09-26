use super::*;
use crate::server::ai::client::Client;
use crate::server::ai::config::{BrowserConfig, Cost, ToolsConfig};
use crate::server::entry::Entry;
use crate::server::test_gateway::{fake_gateway, sse};
use crate::server::wire::{decode, encode, line_len, Decoded};
use std::os::unix::net::UnixStream;

/// 消息里的转录条目（块 + 活尾巴拍平）。这些测试看内容，不看窗口的键。
fn msg_entries(m: &ServerMsg) -> Vec<&Entry> {
    match m {
        ServerMsg::Transcript { blocks, live } | ServerMsg::Blocks { blocks, live } => blocks
            .iter()
            .flat_map(|b| b.entries.iter())
            .chain(live.iter())
            .collect(),
        ServerMsg::OlderBlocks { blocks } | ServerMsg::NewerBlocks { blocks } => {
            blocks.iter().flat_map(|b| b.entries.iter()).collect()
        }
        ServerMsg::EntryMany { entries } => entries.iter().collect(),
        ServerMsg::Entry { entry } => vec![entry],
        _ => Vec::new(),
    }
}

fn tmp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("mypi-daemon-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn spec(base: &str) -> SessionSpec {
    SessionSpec {
        client: Client::new(base, "k", "m"),
        system_prompt: "s".into(),
        max_tokens: 4096,
        cost: Cost::default(),
        context_window: 0,
        model_name: "m".into(),
        cwd: std::env::temp_dir(),
        tool_filter: None,
        roster_source: None,
        tools: ToolsConfig::default(),
        browser: BrowserConfig::default(),
        stream_mode: crate::server::ai::config::StreamMode::default(),
            compact: Default::default(),
            commands: Default::default(),
    }
}

/// A blocking test client: connects, handshakes, and can either read messages
/// or ignore them. `unread` is the honest "slow peer" — bytes pile up in the
/// socket buffer and nothing drains them.
struct TestClient {
    stream: UnixStream,
    buf: Vec<u8>,
    /// Messages read so far (only updated by `read_msg`).
    pub read: Vec<ServerMsg>,
}

impl TestClient {
    fn connect(path: &std::path::Path) -> Self {
        let stream = UnixStream::connect(path).expect("connect");
        stream.set_read_timeout(Some(std::time::Duration::from_secs(10))).unwrap();
        let mut c = Self { stream, buf: Vec::new(), read: Vec::new() };
        c.send(&ClientMsg::Hello { proto: PROTO_VERSION, client: "t".into() });
        assert_eq!(c.read_msg(), ServerMsg::HelloOk {
            proto: PROTO_VERSION,
            commands: crate::server::wire::command_table(),
        });
        c
    }

    fn send(&mut self, msg: &ClientMsg) {
        self.stream.write_all(&encode(msg)).expect("write");
        self.stream.flush().expect("flush");
    }

    fn read_msg(&mut self) -> ServerMsg {
        loop {
            if let Some(n) = line_len(&self.buf) {
                let line: Vec<u8> = self.buf.drain(..n).collect();
                match decode::<ServerMsg>(&line).expect("valid server line") {
                    Decoded::Msg(m) => {
                        self.read.push(m.clone());
                        return m;
                    }
                    Decoded::Partial => unreachable!(),
                }
            }
            let mut tmp = [0u8; 65536];
            let n = match self.stream.read(&mut tmp) {
                Ok(n) => n,
                // 10s read timeout expired: the daemon never answered — a
                // real deadlock, fail loudly with the buffer contents.
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    panic!("read timeout; buffered: {:#?}", &self.buf[..self.buf.len().min(2000)])
                }
                Err(e) => panic!("read failed: {e}"),
            };
            if n == 0 {
                // EOF: whatever is buffered still decodes; past that the peer
                // is really gone.
                if let Some(n) = line_len(&self.buf) {
                    let line: Vec<u8> = self.buf.drain(..n).collect();
                    match decode::<ServerMsg>(&line).expect("valid server line") {
                        Decoded::Msg(m) => {
                            self.read.push(m.clone());
                            return m;
                        }
                        Decoded::Partial => unreachable!(),
                    }
                }
                panic!("daemon closed the connection with nothing buffered");
            }
            self.buf.extend_from_slice(&tmp[..n]);
        }
    }

    /// Read until a message matching `pred` arrives (draining others).
    fn wait_for(&mut self, pred: impl Fn(&ServerMsg) -> bool) -> ServerMsg {
        loop {
            let m = self.read_msg();
            if pred(&m) {
                return m;
            }
        }
    }

    fn attached_id(&mut self) -> i64 {
        match self.wait_for(|m| matches!(m, ServerMsg::Attached { .. })) {
            ServerMsg::Attached { session_id } => session_id,
            other => panic!("expected attached, got {other:?}"),
        }
    }

    /// Politely end the session: daemon flushes and exits.
    fn quit(&mut self) {
        self.send(&ClientMsg::Quit);
        // ServerMsg::HelloOk is the ack (reuses the handshake shape).
        let _ = self.read_msg();
    }

    fn count(&self, pred: impl Fn(&ServerMsg) -> bool) -> usize {
        self.read.iter().filter(|m| pred(m)).count()
    }
}

/// 惰性历史端到端：尾巴先到（第一帧才有得快），更老的按页补，拼起来必须
/// 等于完整转录——少一条就是"历史凭空缺了一截"，而且不报错。
#[test]
fn lazy_history_ships_the_tail_then_the_older_pages_on_demand() {
    use crate::server::entry::Entry;
    use crate::server::store::Store;
    let name = "lazy-e2e";
    // **先起 daemon**：`tmp()` 会把目录清空，先写库会被它删掉。
    let (path, h) = start_daemon_with_idle(
        name,
        "http://127.0.0.1:1/v1",
        std::time::Duration::from_secs(600),
    );
    // 直接拼路径，**不调 `tmp()`**——它会把目录连同 socket 一起删掉。
    let db = std::env::temp_dir()
        .join(format!("mypi-daemon-{name}-{}", std::process::id()))
        .join("sessions.sqlite3");
    let sid = {
        let mut st = Store::open(&db).unwrap();
        let sid = st
            .create_session("2026-09-26 09:00:00", "/tmp")
            .unwrap();
        let es: Vec<Entry> = (0..500)
            .map(|i| Entry::User {
                content: format!("m{i}"),
            })
            .collect();
        st.append(sid, &es).unwrap();
        sid
    };

    let mut c = TestClient::connect(&path);
    c.send(&ClientMsg::Attach { id: sid });

    let tail = match c.wait_for(|m| matches!(m, ServerMsg::Transcript { .. })) {
        ServerMsg::Transcript { blocks, live } => {
            assert!(live.is_empty(), "没有回合在跑，活尾巴该是空的");
            blocks
        }
        _ => unreachable!(),
    };
    assert!(!tail.is_empty(), "尾巴不该是空的");
    assert!(
        tail.len() <= crate::server::hub::SessionHub::HISTORY_TAIL,
        "尾巴不该超过 HISTORY_TAIL，实际 {}",
        tail.len()
    );
    // 快照发完（`State` 收尾）之前，**没有**任何更老的块自己冒上来：
    // 按需 = 不开口不给。
    loop {
        match c.read_msg() {
            ServerMsg::State { .. } => break,
            ServerMsg::OlderBlocks { .. } | ServerMsg::NewerBlocks { .. } => {
                panic!("没人开口要，历史页不该自己来")
            }
            _ => {}
        }
    }
    // 要一段才给一段，直到补齐 500 条，最后用一段空的收尾。边界由前端点名：
    // 每次拿"我手上最老那一块"的 id 开口。
    let mut older: Vec<Entry> = Vec::new();
    let mut edge = tail.first().map(|b| b.id).expect("尾巴非空");
    let mut terminated = false;
    for _ in 0..8 {
        c.send(&ClientMsg::NeedOlder {
            before: edge,
            count: 64,
        });
        let page = match c.wait_for(|m| matches!(m, ServerMsg::OlderBlocks { .. })) {
            ServerMsg::OlderBlocks { blocks } => blocks,
            _ => unreachable!(),
        };
        if page.is_empty() {
            terminated = true;
            break;
        }
        edge = page.first().map(|b| b.id).expect("一段非空");
        // 一段比一段老：拼装时前置，最老的才排在最前面。
        let mut page: Vec<Entry> = page.into_iter().flat_map(|b| b.entries).collect();
        page.extend(older);
        older = page;
    }
    assert!(terminated, "上面没有了应当用一段空的收尾");

    let mut all = older;
    all.extend(tail.into_iter().flat_map(|b| b.entries));
    let texts: Vec<String> = all
        .iter()
        .map(|e| match e {
            Entry::User { content } => content.clone(),
            _ => String::new(),
        })
        .collect();
    assert_eq!(texts.len(), 500, "要完之后应当是完整转录");
    assert_eq!(texts[0], "m0", "最老那条没在最前面");
    assert_eq!(texts[499], "m499");
    c.quit();
    let _ = h.join();
}

/// 前端每收到一帧 `Stream` 就要付一次整帧重绘（~3 ms），所以"画面没变"的
/// 快照不该发。判据是**逐字段**比：工具输出的尾巴滑动时长度可以不变，
/// 按长度比就会把真正的新内容吞掉。
#[test]
fn an_unchanged_stream_snapshot_is_not_shipped() {
    use crate::server::events::StreamView;
    let base = StreamView {
        active: true,
        text: "半句".into(),
        tool_output: "line1\nline2\n".into(),
        ..Default::default()
    };
    // 一模一样：不发。
    assert!(!Daemon::stream_changed(Some(&base), &base.clone()));
    // 正文长了一截：发。
    let grown = StreamView {
        text: "半句话".into(),
        ..base.clone()
    };
    assert!(Daemon::stream_changed(Some(&base), &grown));
    // **同样长度、不同内容**（尾巴滑了一行）：必须发。
    let slid = StreamView {
        tool_output: "line2\nline3\n".into(),
        ..base.clone()
    };
    assert!(Daemon::stream_changed(Some(&base), &slid));
    // 第一次（没有上一帧）：发。
    assert!(Daemon::stream_changed(None, &base));
}

fn start_daemon(name: &str, base: &str) -> (std::path::PathBuf, std::thread::JoinHandle<()>) {
    start_daemon_with_idle(name, base, std::time::Duration::from_secs(600))
}

fn start_daemon_with_idle(
    name: &str,
    base: &str,
    idle: std::time::Duration,
) -> (std::path::PathBuf, std::thread::JoinHandle<()>) {
    let dir = tmp(name);
    let path = dir.join("d.sock");
    let hub = SessionHub::new(dir.join("sessions.sqlite3"));
    let daemon = Daemon::bind(path.clone(), hub, spec(base), idle).unwrap();
    let h = std::thread::spawn(move || daemon.serve().unwrap());
    // Wait for the socket to exist.
    for _ in 0..200 {
        if path.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(path.exists(), "daemon socket never appeared");
    (path, h)
}

#[test]
fn hello_version_mismatch_is_refused() {
    let (base, _seen) = fake_gateway(vec![]);
    let (path, _h) = start_daemon("hello-mismatch", &base);
    let mut c = TestClient::connect(&path);
    c.send(&ClientMsg::Hello { proto: PROTO_VERSION + 1, client: "old".into() });
    match c.read_msg() {
        ServerMsg::Error { code, .. } => assert_eq!(code, ErrorCode::ProtoMismatch),
        other => panic!("expected proto_mismatch, got {other:?}"),
    }
    // The daemon then refuses the connection: the socket must close (EOF) —
    // a half-open refused peer would hang every well-behaved client.
    let mut tmp_buf = [0u8; 64];
    let n = c.stream.read(&mut tmp_buf).expect("read after refusal");
    assert_eq!(n, 0, "daemon must close the refused connection");
    // The 600s idle limit means the daemon thread will not exit on its own;
    // dropping it with the test process is the honest end here (a real
    // daemon exits by idle or quit, both covered by other tests).
    let _ = std::fs::remove_dir_all(tmp("hello-mismatch"));
}

#[test]
fn draft_submit_creates_session_and_delivers_transcript() {
    let (base, seen) = fake_gateway(vec![sse(&["你好"])]); // one round, no tools
    let (path, h) = start_daemon("draft-submit", &base);

    let mut c = TestClient::connect(&path);
    c.send(&ClientMsg::Submit { text: "问".into() });
    let id = c.attached_id();
    // Read until the assistant's reply lands (full snapshot or tail — both
    // legal; stream frames may interleave). The reply entry IS the round's
    // finish as far as the front end is concerned.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        assert!(std::time::Instant::now() < deadline, "round never finished");
        let m = c.read_msg();
        if msg_entries(&m)
            .iter()
            .any(|e| matches!(e, Entry::Assistant { .. }))
        {
            break;
        }
    }
    let bodies = seen.join().unwrap();
    assert_eq!(bodies.len(), 1);
    assert!(bodies[0].contains("\"model\": \"m\""), "gateway saw the request: {}", bodies[0]);

    // The reply reached this client: exactly one full snapshot arrived whose
    // entries carry both the user text and the model reply.
    let entries: Vec<&Entry> = c.read.iter().flat_map(msg_entries).collect();
    assert!(
        entries.iter().any(|e| matches!(e, Entry::User { content } if content == "问"))
            && entries.iter().any(|e| matches!(e, Entry::Assistant { content, .. } if content == "你好")),
        "user text and reply must both have shipped; got {entries:?}"
    );

    // A second client attaches to the same session and gets a full snapshot.
    let mut c2 = TestClient::connect(&path);
    c2.send(&ClientMsg::Attach { id });
    c2.attached_id();
    let deadline2 = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        assert!(std::time::Instant::now() < deadline2, "attach snapshot never arrived");
        let m = c2.read_msg();
        if matches!(&m, ServerMsg::Transcript { blocks, .. } if !blocks.is_empty()) {
            break;
        }
    }
    // Attach ships a snapshot set: transcript + stream + state in some order.
    let deadline3 = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !(c2.read.iter().any(|m| matches!(m, ServerMsg::Stream { .. }))
        && c2.read.iter().any(|m| matches!(m, ServerMsg::State { .. })))
    {
        assert!(std::time::Instant::now() < deadline3, "attach snapshots incomplete: {:#?}", c2.read);
        let _ = c2.read_msg();
    }

    c.quit();
    h.join().unwrap();
    let _ = std::fs::remove_dir_all(tmp("draft-submit"));
}

#[test]
fn two_sessions_two_front_ends_stay_separate() {
    let (base, _seen) = fake_gateway(vec![sse(&["一"]), sse(&["二"])]);
    let (path, h) = start_daemon("two-sessions", &base);

    let mut a = TestClient::connect(&path);
    a.send(&ClientMsg::Submit { text: "甲".into() });
    let id_a = a.attached_id();

    let mut b = TestClient::connect(&path);
    b.send(&ClientMsg::Submit { text: "乙".into() });
    let id_b = b.attached_id();
    assert_ne!(id_a, id_b, "two drafts must become two sessions");

    // Each front end receives exactly the entries of its own session: the
    // user text it sent (echoed through the transcript) and nothing from the
    // other session's transcript.
    a.wait_for(|m| matches!(m, ServerMsg::State { busy: false, .. }));
    b.wait_for(|m| matches!(m, ServerMsg::State { busy: false, .. }));
    let texts_of = |msgs: &TestClient| -> Vec<String> {
        msgs.read
            .iter()
            .flat_map(msg_entries)
            .filter_map(|e| match e {
                Entry::User { content } | Entry::Assistant { content, .. } => Some(content.clone()),
                _ => None,
            })
            .collect()
    };
    let ta = texts_of(&a);
    let tb = texts_of(&b);
    assert!(ta.contains(&"甲".to_string()), "A sees its own text: {ta:?}\nA raw: {:?}", a.read);
    assert!(tb.contains(&"乙".to_string()), "B sees its own text: {tb:?}\nB raw: {:?}", b.read);
    assert!(!ta.contains(&"乙".to_string()) && !tb.contains(&"甲".to_string()), "no cross-talk\nA: {:?}\nB: {:?}", a.read, b.read);

    a.quit();
    b.quit();
    h.join().unwrap();
    let _ = std::fs::remove_dir_all(tmp("two-sessions"));
}

#[test]
fn a_front_end_that_stops_reading_never_stalls_the_round() {
    // THE backpressure test (SERVER.md §9 step 2): one client reads nothing
    // while a full round completes for another. Recovery must be complete.
    let (base, _seen) = fake_gateway(vec![sse(&["第", "一", "个", "字"])]);
    let (path, h) = start_daemon("backpressure", &base);

    let mut fast = TestClient::connect(&path);
    fast.send(&ClientMsg::Submit { text: "问".into() });
    let id = fast.attached_id();

    // The dead weight: attaches, then never reads a byte.
    let mut dead = TestClient::connect(&path);
    dead.send(&ClientMsg::Attach { id });
    // Give the daemon a moment to start shipping to `dead`, then let the
    // round finish while `dead` keeps its mouth shut.
    fast.wait_for(|m| matches!(m, ServerMsg::State { busy: false, .. }));

    // The fast client got the whole reply.
    let got: Vec<String> = fast
        .read
        .iter()
        .flat_map(|m| match m {
            ServerMsg::Stream { text, active: true, .. } => vec![text.clone()],
            _ => vec![],
        })
        .collect();
    assert!(
        got.iter().any(|t| t.contains("第一个字")) || fast.count(|m| matches!(m, ServerMsg::Transcript { .. })) > 0,
        "fast client must see the reply; got {got:?}"
    );

    // Now the slow one wakes up and reads: it must receive everything that
    // piled up (its socket buffer held it; the write queue ordered it).
    dead.stream.set_read_timeout(Some(std::time::Duration::from_secs(5))).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let mut saw_reply = false;
    while std::time::Instant::now() < deadline {
        let m = dead.read_msg();
        if msg_entries(&m).iter().any(|e| matches!(e, Entry::Assistant { content, .. } if content.contains("第一个字"))) {
            saw_reply = true;
            break;
        }
    }
    assert!(saw_reply, "the recovering client must eventually see the full reply");

    fast.quit();
    dead.quit();
    h.join().unwrap();
    let _ = std::fs::remove_dir_all(tmp("backpressure"));
}

#[test]
fn list_sessions_and_rounds_answer_from_the_db() {
    let (base, _seen) = fake_gateway(vec![sse(&["回"])] );
    let (path, h) = start_daemon("list", &base);

    let mut c = TestClient::connect(&path);
    c.send(&ClientMsg::Submit { text: "问".into() });
    let id = c.attached_id();
    c.wait_for(|m| matches!(m, ServerMsg::State { busy: false, .. }));

    c.send(&ClientMsg::ListSessions { under: None });
    let sessions = match c.wait_for(|m| matches!(m, ServerMsg::Sessions { .. })) {
        ServerMsg::Sessions { sessions } => sessions,
        other => panic!("{other:?}"),
    };
    assert!(sessions.iter().any(|s| s.id == id));

    c.send(&ClientMsg::ListRounds { id });
    let rounds = match c.wait_for(|m| matches!(m, ServerMsg::Rounds { .. })) {
        ServerMsg::Rounds { rounds, .. } => rounds,
        other => panic!("{other:?}"),
    };
    assert_eq!(rounds.len(), 1, "one submitted round = one stored header");
    assert_eq!(rounds[0].model, "m");
    assert_eq!(rounds[0].stop_reason.as_deref(), Some("stop"));

    // Replay answers with the rebuilt request.
    c.send(&ClientMsg::Replay { id, round: rounds[0].seq });
    match c.wait_for(|m| matches!(m, ServerMsg::Replay { .. })) {
        ServerMsg::Replay { replay, .. } => {
            assert_eq!(replay.model, "m");
            assert_eq!(replay.body["model"], "m");
        }
        other => panic!("{other:?}"),
    }

    // Unknown ids answer as errors, never panics: an unknown session has no
    // rounds (empty answer, not a failure), but attaching must refuse.
    c.send(&ClientMsg::ListRounds { id: 9999 });
    match c.wait_for(|m| matches!(m, ServerMsg::Rounds { .. })) {
        ServerMsg::Rounds { id, rounds } => {
            assert_eq!(id, 9999);
            assert!(rounds.is_empty());
        }
        other => panic!("expected empty rounds, got {other:?}"),
    }
    c.send(&ClientMsg::Attach { id: 9999 });
    assert!(matches!(
        c.wait_for(|m| matches!(m, ServerMsg::Error { code: ErrorCode::NoSuchSession, .. })),
        ServerMsg::Error { code: ErrorCode::NoSuchSession, .. }
    ));

    c.quit();
    h.join().unwrap();
    let _ = std::fs::remove_dir_all(tmp("list"));
}

#[test]
fn idle_daemon_exits_by_itself() {
    let (base, _seen) = fake_gateway(vec![]);
    let (path, h) = start_daemon_with_idle("idle-exit", &base, std::time::Duration::from_millis(100));
    let _unused = TestClient::connect(&path); // attach nothing; drop it at once
    drop(TestClient::connect(&path));
    let started = std::time::Instant::now();
    // idle_limit is 100ms in the fixture; the daemon must exit on its own.
    let _ = h.join();
    assert!(
        started.elapsed() < std::time::Duration::from_secs(5),
        "daemon lingered after everyone left"
    );
    let _ = std::fs::remove_dir_all(tmp("idle-exit"));
}

#[test]
fn state_frame_carries_model_metadata() {
    // 展示名规则（name 优先，缺省 id）与上下文窗口都由服务器下发；
    // 前端不读 models.yml（SERVER.md §0 的边界）。
    let dir = tmp("meta");
    let path = dir.join("d.sock");
    let hub = SessionHub::new(dir.join("sessions.sqlite3"));
    let mut sp = spec("http://127.0.0.1:1/v1");
    sp.model_name = "GLM5.3F".into();
    sp.context_window = 128_000;
    let daemon = Daemon::bind(
        path.clone(),
        hub,
        sp,
        std::time::Duration::from_secs(600),
    )
    .unwrap();
    let h = std::thread::spawn(move || daemon.serve().unwrap());
    for _ in 0..200 {
        if path.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let mut c = TestClient::connect(&path);
    c.send(&ClientMsg::Submit { text: "问".into() });
    let _id = c.attached_id();
    c.wait_for(|m| matches!(m, ServerMsg::State { busy: false, .. }));
    let st = c
        .read
        .iter()
        .find_map(|m| match m {
            ServerMsg::State {
                model,
                context_window,
                ..
            } => Some((model.clone(), *context_window)),
            _ => None,
        })
        .expect("a state frame must have arrived");
    assert_eq!(st.0, "GLM5.3F", "display name comes from models.yml name");
    assert_eq!(st.1, 128_000, "context window rides the wire");

    c.quit();
    h.join().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

// ---- snapshot chunking ----------------------------------------------------
//
// One JSON line per message, and a long transcript does not fit in one:
// 32 000 entries encode to ~39 MB, over MAX_LINE_BYTES. The snapshot ships as
// a run of messages instead — `Transcript` first (it replaces), `EntryMany`
// after (they append) — which is what the view already does with them.

/// 用户自己那条消息必须在**提交那一刻**就发出去，不能等到回合里有了别的
/// 事件才搭车：它由 `Session::start_turn` 直接写进转录，一个 `Change` 都不
/// 发（`start_turn` 报的是 `Change::Stream`），所以「尾巴按事件发」的账本
/// 根本不知道它来了——结果就是按回车后屏幕上什么都不动，直到模型开口，
/// 用户消息和回复一起冒出来。
#[test]
fn the_submitters_own_message_ships_before_the_round_does_anything() {
    let (base, _seen) = fake_gateway(vec![sse(&["你好"])]);
    let (path, h) = start_daemon("echo-on-submit", &base);

    let mut c = TestClient::connect(&path);
    c.send(&ClientMsg::Submit { text: "问".into() });
    let _ = c.attached_id();

    // 第一条带转录内容的消息：只许有用户那条，不许已有回复/工具。
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let first = loop {
        assert!(std::time::Instant::now() < deadline, "转录一直没发出来");
        match c.read_msg() {
            m @ (ServerMsg::Transcript { .. }
            | ServerMsg::Blocks { .. }
            | ServerMsg::EntryMany { .. }
            | ServerMsg::Entry { .. }) => break m,
            _ => continue,
        }
    };
    let entries: Vec<&Entry> = msg_entries(&first);
    assert!(
        entries.iter().any(|e| matches!(e, Entry::User { content } if content == "问")),
        "第一条转录里没有用户那条：{entries:?}"
    );
    assert!(
        !entries.iter().any(|e| matches!(
            e,
            Entry::Assistant { .. } | Entry::ToolRequest { .. } | Entry::ToolResult { .. }
        )),
        "用户消息搭了回合事件的便车（回车后屏幕上不动的原因）：{entries:?}"
    );

    c.quit();
    h.join().unwrap();
    let _ = std::fs::remove_dir_all(tmp("echo-on-submit"));
}

#[test]
fn an_empty_snapshot_still_says_so() {
    // "You have nothing" is information: the view must clear.
    // 空快照 = 空窗口 + 空活尾巴：前端据此清干净（"你没有东西"也是信息）。
    let empty = ServerMsg::Transcript {
        blocks: Vec::new(),
        live: Vec::new(),
    };
    let bytes = encode(&empty);
    match decode::<ServerMsg>(&bytes).unwrap() {
        Decoded::Msg(ServerMsg::Transcript { blocks, live }) => {
            assert!(blocks.is_empty() && live.is_empty());
        }
        other => panic!("{other:?}"),
    }
}
