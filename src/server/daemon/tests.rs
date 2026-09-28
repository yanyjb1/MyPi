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
        model_provider: "test".into(),
        currency: crate::server::ai::config::Currency::Cny,
        cwd: std::env::temp_dir(),
        tool_filter: None,
        roster_source: None,
        tools: ToolsConfig::default(),
        browser: BrowserConfig::default(),
        stream_mode: crate::server::ai::config::StreamMode::default(),
            compact: Default::default(),
            compact_instruction: None,
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

/// 一个**分段发货**的网关：正文那半句先到，工具调用隔 `delay` 才来。
///
/// [`fake_gateway`] 用 `Content-Length` 一次 write 把整个响应送出去——每个
/// chunk 挤在同一瞬间抵达，于是"半句到达"和"正文被清空"落在同一个 pump
/// 节拍里被合并掉（初始态和终态一样，本来也不该发帧）。可要测的正是中间
/// 那一帧，所以这里把两次写隔开。
fn staged_gateway(delay: std::time::Duration) -> (String, std::thread::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let h = std::thread::spawn(move || {
        // 第一轮：正文先走。
        let Ok((mut sock, _)) = listener.accept() else {
            return;
        };
        let _ = crate::server::test_gateway::read_request(&mut sock);
        let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n");
        let _ = sock.write_all(b"data: {\"choices\":[{\"delta\":{\"content\":\"\xe6\x88\x91\xe5\x85\x88\xe7\x9c\x8b\xe7\x9c\x8b\"}}]}\n\n");
        let _ = sock.flush();
        std::thread::sleep(delay);
        // 隔一拍，工具调用才来。
        let _ = sock.write_all(
            b"data: {\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_1\",\"type\":\"function\",\"function\":{\"name\":\"bash\",\"arguments\":\"{\\\"command\\\":\\\"sleep 0.4\\\",\\\"intent\\\":\\\"wait a beat\\\"}\"}}]},\"finish_reason\":\"tool_calls\"}]}\n\n",
        );
        let _ = sock.write_all(b"data: [DONE]\n\n");
        let _ = sock.flush();
        drop(sock);

        // 第二轮：收尾回复。
        let Ok((mut sock, _)) = listener.accept() else {
            return;
        };
        let _ = crate::server::test_gateway::read_request(&mut sock);
        let _ = sock.write_all(b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\n\r\n");
        let _ = sock.write_all(b"data: {\"choices\":[{\"delta\":{\"content\":\"\xe8\xaf\xbb\xe5\xae\x8c\xe4\xba\x86\"}}]}\n\n");
        let _ = sock.write_all(b"data: [DONE]\n\n");
        let _ = sock.flush();
    });
    (format!("http://{addr}/v1"), h)
}

/// 工具开始的那一刻，服务端会清空正文缓冲（那半句已经随工具条目存档了）。
/// **这个清空必须发一帧出去**：不发的话，前端把那半句继续挂在正文区，于是
/// 屏幕上同一段字出现两份——工具卡片上方一份（来自条目），正文区一份（来自
/// 早先那帧流式快照），直到下一轮内容把它顶掉。
///
/// 它报的是 `Change::Transcript`（同时还有一条工具条目要落），所以只认
/// `Change::Stream` 的发送逻辑一定会漏掉它。
#[test]
fn the_cleared_text_buffer_reaches_the_front_end_as_a_stream_frame() {
    let dir = tmp("tool-clear");
    let path = dir.join("d.sock");
    // 工具**慢一点**：回合快到一个 tick 里跑完的话，中间那几帧会被合并掉
    // （初始态和终态一样，本来就不该发），这里要看的正是中间那一帧。
    let (base, _srv) = staged_gateway(std::time::Duration::from_millis(150));
    let hub = SessionHub::new(dir.join("sessions.sqlite3"));
    let daemon = Daemon::bind(
        path.clone(),
        hub,
        spec(&base),
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
    c.send(&ClientMsg::Submit {
        text: "看看文件".into(),
        from_block: None,
    });
    let _id = c.attached_id();
    // 那半句先到。
    c.wait_for(|m| matches!(m, ServerMsg::Stream { text, .. } if text == "我先看看"));
    // 工具一开始，服务端就把那半句从流式槽位取走（它已经随工具条目存档）。
    // 这一帧到不了的话，前端会把它继续挂在正文区，和工具卡片上的同一段字
    // 重成两份——等不到就在这里超时炸掉，那正是要防的退化。
    c.wait_for(|m| matches!(m, ServerMsg::Stream { text, .. } if text.is_empty()));

    c.quit();
    h.join().unwrap();
    let _ = std::fs::remove_dir_all(dir);
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
    c.send(&ClientMsg::Submit { text: "问".into(), from_block: None });
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
    a.send(&ClientMsg::Submit { text: "甲".into(), from_block: None });
    let id_a = a.attached_id();

    let mut b = TestClient::connect(&path);
    b.send(&ClientMsg::Submit { text: "乙".into(), from_block: None });
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
    fast.send(&ClientMsg::Submit { text: "问".into(), from_block: None });
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
    c.send(&ClientMsg::Submit { text: "问".into(), from_block: None });
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
    c.send(&ClientMsg::Submit { text: "问".into(), from_block: None });
    let _id = c.attached_id();
    c.wait_for(|m| matches!(m, ServerMsg::State { busy: false, .. }));
    let st = c
        .read
        .iter()
        .find_map(|m| match m {
            ServerMsg::State {
                model,
                context_window,
                ctx_estimate,
                ..
            } => Some((model.clone(), *context_window, *ctx_estimate)),
            _ => None,
        })
        .expect("a state frame must have arrived");
    assert_eq!(st.0, "GLM5.3F", "display name comes from models.yml name");
    assert_eq!(st.1, 128_000, "context window rides the wire");
    assert!(
        st.2 > 0,
        "还没跑过回合时用量表要有起点（系统提示词加工具手册的折算）"
    );

    c.quit();
    h.join().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

/// 提交那一刻状态帧就得走线：状态栏的 spinner 靠 `busy` 的**起始沿**亮起，
/// 而提交只报 `Change::Stream`——状态帧要是非等到 `TurnDone` 才发，它整场
/// 都亮不起来（这正是它此前死活不转的原因）。
#[test]
fn busy_reaches_the_front_end_the_moment_a_turn_starts() {
    let dir = tmp("busy-edge");
    let path = dir.join("d.sock");
    let (base, _srv) = fake_gateway(vec![sse(&["答"])]);
    let hub = SessionHub::new(dir.join("sessions.sqlite3"));
    let daemon = Daemon::bind(
        path.clone(),
        hub,
        spec(&base),
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
    c.send(&ClientMsg::Submit {
        text: "问".into(),
        from_block: None,
    });
    let _id = c.attached_id();
    c.wait_for(|m| matches!(m, ServerMsg::State { busy: false, .. }));
    assert!(
        c.read
            .iter()
            .any(|m| matches!(m, ServerMsg::State { busy: true, .. })),
        "回合进行中的状态帧没发出去，spinner 没有起始沿：{:?}",
        c.read
            .iter()
            .filter(|m| matches!(m, ServerMsg::State { .. }))
            .collect::<Vec<_>>()
    );

    c.quit();
    h.join().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}

/// `/name` 这类命令直接改会话字段，**不产生 `SessionEvent`**——delta pass
/// 看不见它们。状态帧改成按内容比对之后，新名字一个 tick 内就到前端，不必
/// 等这一轮跑完（那是用户抱怨的"命令没有及时反馈"）。
#[test]
fn renaming_reaches_the_front_end_without_waiting_for_a_turn() {
    let dir = tmp("name-now");
    let path = dir.join("d.sock");
    let hub = SessionHub::new(dir.join("sessions.sqlite3"));
    let daemon = Daemon::bind(
        path.clone(),
        hub,
        spec("http://127.0.0.1:1/v1"),
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
    c.send(&ClientMsg::Command {
        name: "/name".into(),
        args: "改个名".into(),
    });
    c.wait_for(
        |m| matches!(m, ServerMsg::State { name: Some(n), .. } if n == "改个名"),
    );

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
    c.send(&ClientMsg::Submit { text: "问".into(), from_block: None });
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


/// `/usage`：跑过一轮之后敲它，服务端把**按轮**记下的用量聚起来、拿当前价格表
/// 折成钱，并且只回给发起命令的那个前端。
///
/// 断言落到四个数上（输入/输出 token、两栏的钱），因为那正是用户要看的东西：
/// 账单算错一位小数和一整块面板画错是两回事，这里钉住的是前者。
#[test]
fn usage_answers_the_asking_front_end_with_the_session_bill() {
    use crate::server::commands::{CommandEnv, ModelSwitch};
    use crate::server::wire::UsageLang;
    let dir = tmp("usage-panel");
    let path = dir.join("d.sock");
    let (base, _srv) = fake_gateway(vec![sse(&["答"])]);
    let hub = SessionHub::new(dir.join("sessions.sqlite3"));
    let mut sp = spec(&base);
    // 价格表：输入 1/M、输出 4/M、缓存读 0.02/M。测的是"token × 单价"，
    // 换算式本身由 pricing 的测试钉着。
    sp.cost = Cost {
        input: 1.0,
        output: 4.0,
        cache_read: 0.02,
        cache_write: 0.0,
    };
    sp.commands = CommandEnv {
        resolve_model: Some(std::sync::Arc::new(|_: &str| {
            Ok(ModelSwitch {
                base_url: "http://127.0.0.1:1/v1".into(),
                api_key: "k".into(),
                model_id: "m".into(),
                provider: "test".into(),
                name: "FAKE-1M".into(),
                context_window: 1_000_000,
                max_tokens: 4096,
                compat: None,
                cost: Cost {
                    input: 1.0,
                    output: 4.0,
                    cache_read: 0.02,
                    cache_write: 0.0,
                },
                // 用美元计价：面板要按汇率折到人民币，这张表的数字才可比。
                currency: crate::server::ai::config::Currency::Usd,
            })
        })),
        fx: crate::server::ai::config::FxConfig { usd_cny: 7.2 },
        ..Default::default()
    };
    let daemon = Daemon::bind(path.clone(), hub, sp, std::time::Duration::from_secs(600)).unwrap();
    let h = std::thread::spawn(move || daemon.serve().unwrap());
    for _ in 0..200 {
        if path.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }

    let mut c = TestClient::connect(&path);
    c.send(&ClientMsg::Submit {
        text: "问".into(),
        from_block: None,
    });
    let _ = c.attached_id();
    c.wait_for(|m| matches!(m, ServerMsg::State { busy: false, .. }));

    c.send(&ClientMsg::Command {
        name: "/usage".into(),
        args: "lang:zh".into(),
    });
    let groups = match c.wait_for(|m| matches!(m, ServerMsg::Usage { .. })) {
        ServerMsg::Usage { lang, groups } => {
            assert_eq!(lang, UsageLang::Zh, "lang:zh 要真的换口径");
            groups
        }
        other => panic!("expected usage, got {other:?}"),
    };

    assert_eq!(groups.len(), 1, "只问过一个模型：{groups:?}");
    let g = &groups[0];
    assert_eq!(g.rounds, 1, "一轮就是一轮");
    // fake_gateway 的 usage chunk：prompt 10 / completion 2 / 无 cached。
    assert_eq!(g.prompt_tokens, 10);
    assert_eq!(g.completion_tokens, 2);
    assert_eq!(g.cached_tokens, 0);
    assert_eq!(g.name, "FAKE-1M", "展示名来自当前配置，不是裸 id");
    assert_eq!(
        g.currency.as_deref(),
        Some("¥"),
        "跨币种的账要折到同一个单位上，不然表里的数字没法比"
    );
    let cost = g.cost.expect("有价格表就该算得出钱");
    // $1 = ¥7.2：输入 10 token × $1/M、输出 2 token × $4/M，再各乘 7.2。
    assert!((cost.input - 10.0 / 1e6 * 7.2).abs() < 1e-12, "{cost:?}");
    assert!((cost.output - 8.0 / 1e6 * 7.2).abs() < 1e-12, "{cost:?}");
    assert_eq!(cost.cache_read, 0.0);
    assert_eq!(cost.cache_write, 0.0);

    // 面板**不是**转录条目：进历史就等于把它写进了模型看得见的上下文。
    for m in &c.read {
        for e in msg_entries(m) {
            let text = format!("{e:?}");
            assert!(!text.contains("FAKE-1M"), "账单不许混进转录：{text}");
        }
    }

    c.quit();
    h.join().unwrap();
    let _ = std::fs::remove_dir_all(dir);
}
