use super::*;

// ---- round trips ----------------------------------------------------------

fn roundtrip<M>(msg: M)
where
    M: serde::Serialize + serde::de::DeserializeOwned + PartialEq + Clone + std::fmt::Debug,
{
    let bytes = encode(&msg);
    assert_eq!(bytes.last(), Some(&b'\n'), "encoded messages end with \\n");

    // A message split into arbitrary byte batches still decodes, and the
    // buffer's tail (the next message's bytes) is never consumed.
    for split in 1..bytes.len() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&bytes[..split]);
        let mut decoded = None;
        for _ in 0..2 {
            match decode::<M>(&buf).expect("partial input must not error") {
                Decoded::Msg(m) => {
                    decoded = Some(m);
                    break;
                }
                Decoded::Partial => {
                    buf.extend_from_slice(&bytes[split..]);
                }
            }
        }
        assert_eq!(decoded, Some(msg.clone()), "split at byte {split}");
        assert_eq!(line_len(&buf), Some(bytes.len()));
    }
}

#[test]
fn hello_roundtrips_through_every_split() {
    roundtrip(ClientMsg::Hello {
        proto: PROTO_VERSION,
        client: "tui".into(),
    });
}

#[test]
fn every_client_msg_roundtrips() {
    let msgs = vec![
        ClientMsg::Attach { id: 42 },
        ClientMsg::Detach,
        ClientMsg::ListSessions { under: None },
        ClientMsg::ListRounds { id: 7 },
        ClientMsg::Replay { id: 7, round: 3 },
        ClientMsg::Submit {
            text: "你好\n第二行".into(),
        },
        ClientMsg::Interrupt,
        ClientMsg::Logs { limit: 50 },
        ClientMsg::Quit,
    ];
    for m in msgs {
        roundtrip(m);
    }
}

#[test]
fn every_server_msg_roundtrips() {
    let msgs = vec![
        ServerMsg::HelloOk {
            proto: PROTO_VERSION,
            commands: crate::server::wire::command_table(),
        },
        ServerMsg::Attached { session_id: 3 },
        ServerMsg::Transcript {
            blocks: vec![WireBlock {
                id: 7,
                entries: vec![
                    Entry::User { content: "hi".into() },
                Entry::Assistant {
                    content: "reply".into(),
                    usage: None,
                },
                Entry::Reasoning { content: "hmm".into() },
                Entry::ToolRequest {
                    call_id: "c1".into(),
                    name: "bash".into(),
                    args: "{\"cmd\":\"ls\"}".into(),
                    intent: "list files".into(),
                    text: "let me look".into(),
                    first: true,
                },
                Entry::ToolResult {
                    call_id: "c1".into(),
                    name: "bash".into(),
                    ok: true,
                    result: "src".into(),
                    details: None,
                    duration_ms: 0,
                },
                Entry::System {
                    text: "switched model".into(),
                    align: crate::server::entry::Align::Left,
                    pin: false,
                },
                    Entry::Compaction {
                        first_kept_entry: 12,
                        summary: "earlier context".into(),
                    },
                ],
            }],
            live: vec![Entry::User {
                content: "还没落盘".into(),
            }],
        },
        ServerMsg::Blocks {
            blocks: vec![WireBlock {
                id: 8,
                entries: vec![Entry::Assistant {
                    content: "落盘了".into(),
                    usage: None,
                }],
            }],
            live: Vec::new(),
        },
        ServerMsg::OlderBlocks {
            blocks: vec![WireBlock {
                id: 6,
                entries: vec![Entry::User {
                    content: "更老的".into(),
                }],
            }],
        },
        ServerMsg::NewerBlocks {
            blocks: vec![WireBlock {
                id: 9,
                entries: vec![Entry::User {
                    content: "更新的".into(),
                }],
            }],
        },
        ServerMsg::Entry {
            entry: Entry::Error { text: "boom".into() },
        },
        ServerMsg::Stream {
            active: true,
            text: "partial".into(),
            reasoning: "thinking…".into(),
            reasoning_done: false,
            live: LiveActivity::Tool {
                intent: "running tests".into(),
            },
            run_state: RunState::Tool {
                intent: "running tests".into(),
            },
            tool_output: String::new(),
        },
        ServerMsg::State {
            model: "gpt-x".into(),
            name: Some("fix bug".into()),
            cwd: "/tmp".into(),
            spend_usd: 0.42,
            last_prompt_tokens: 1200,
            context_window: 1_000_000,
            busy: true,
        },
        ServerMsg::Sessions {
            sessions: vec![SessionInfo {
                first_message: Some("第一条".into()),
                bytes: 42,
                id: 1,
                name: None,
                started_at: "2026-09-26T00:00:00Z".into(),
                cwd: Some("/home/Arisha".into()),
            }],
        },
        ServerMsg::Rounds {
            id: 1,
            rounds: vec![RoundInfo {
                seq: 1,
                ts: "2026-09-26T00:00:00Z".into(),
                model: "gpt-x".into(),
                protocol: "openai-chat-completions".into(),
                base_url: "https://api.example.com/v1".into(),
                max_tokens: 4096,
                stop_reason: Some("end_turn".into()),
                first_block: Some(2),
                last_block: Some(9),
            }],
        },
        ServerMsg::Logs {
            records: vec![LogRecord {
                at_ms: 1_790_000_000_000u64,
                level: "warn".into(),
                scope: "gpt-x".into(),
                text: "retry 1/3".into(),
            }],
        },
        ServerMsg::Error {
            code: ErrorCode::NoSuchSession,
            message: "no session 99".into(),
        },
    ];
    for m in msgs {
        roundtrip(m);
    }
}

#[test]
fn replay_roundtrips_with_json_payloads() {
    let replay = Replay {
        round: 2,
        model: "gpt-x".into(),
        protocol: "openai-chat-completions".into(),
        base_url: "https://api.example.com/v1".into(),
        system: "You are MyPi.".into(),
        tools_json: "[{\"name\":\"bash\"}]".into(),
        stop_reason: Some("end_turn".into()),
        max_tokens: 2048,
        messages: serde_json::json!([{"role": "user", "content": "hi"}]),
        body: serde_json::json!({"model": "gpt-x", "max_tokens": 2048}),
    };
    roundtrip(ServerMsg::Replay { id: 5, replay });
}

// ---- wire shape guards ----------------------------------------------------

#[test]
fn entry_is_tagged_snake_case_on_the_wire() {
    // The wire format is load-bearing: a front end in another language reads
    // these tags. Pin them so a rename cannot slip through silently.
    let line = encode(&Entry::ToolRequest {
        call_id: "c1".into(),
        name: "bash".into(),
        args: "{}".into(),
        intent: String::new(),
        text: String::new(),
        first: true,
    });
    let text = std::str::from_utf8(&line).unwrap();
    assert_eq!(
        text.trim_end(),
        r#"{"type":"tool_request","call_id":"c1","name":"bash","args":"{}","intent":"","text":"","first":true}"#
    );
    let line = encode(&RunState::Thinking);
    assert_eq!(std::str::from_utf8(&line).unwrap().trim_end(), "\"thinking\"");
}

// ---- framing / error policy ----------------------------------------------

#[test]
fn partial_input_reports_partial_not_error() {
    let bytes = encode(&ClientMsg::Interrupt);
    for n in 0..bytes.len() {
        assert!(matches!(
            decode::<ClientMsg>(&bytes[..n]).expect("no error on prefix"),
            Decoded::Partial
        ));
    }
}

#[test]
fn oversized_line_is_rejected() {
    let long = vec![b'a'; MAX_LINE_BYTES + 1];
    let err = decode::<ClientMsg>(&long).expect_err("over-long line must be refused");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);

    // Same cap even when a newline eventually arrives.
    let mut framed = long;
    framed.push(b'\n');
    let err = decode::<ClientMsg>(&framed).expect_err("over-long line must be refused");
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);

    // At the cap exactly: still accepted (a full transcript is a few MB).
    let at_cap = vec![b'a'; MAX_LINE_BYTES];
    assert!(matches!(
        decode::<ClientMsg>(&at_cap).expect("no error below cap"),
        Decoded::Partial
    ));
}

#[test]
fn bad_json_is_an_error_not_a_panic() {
    for bad in [b"not json\n".as_slice(), b"{\"type\":\"nope\"}\n", b"[]\n"] {
        let err = match decode::<ClientMsg>(bad) {
            Err(e) => e,
            Ok(_) => panic!("malformed line must error"),
        };
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    }
    // ...and the error is per-connection material: decode of a *different*
    // buffer is unaffected, i.e. nothing here is global state.
    assert!(matches!(
        decode::<ClientMsg>(encode(&ClientMsg::Quit).as_slice()),
        Ok(Decoded::Msg(ClientMsg::Quit))
    ));
}

#[test]
fn two_messages_in_one_buffer_decode_in_order() {
    let mut buf = encode(&ClientMsg::Attach { id: 1 });
    buf.extend_from_slice(&encode(&ClientMsg::Detach));
    assert_eq!(line_len(&buf), Some(encode(&ClientMsg::Attach { id: 1 }).len()));
    let first = match decode::<ClientMsg>(&buf).unwrap() {
        Decoded::Msg(m) => m,
        Decoded::Partial => panic!("complete line must decode"),
    };
    assert_eq!(first, ClientMsg::Attach { id: 1 });
    let rest = &buf[line_len(&buf).unwrap()..];
    assert_eq!(
        match decode::<ClientMsg>(rest).unwrap() {
            Decoded::Msg(m) => m,
            Decoded::Partial => panic!("complete line must decode"),
        },
        ClientMsg::Detach
    );
}

#[test]
fn decode_consume_helper_roundtrips_a_stream() {
    // The exact loop a connection reader runs: pull bytes, decode, consume.
    let wire: Vec<u8> = [ClientMsg::Attach { id: 9 }, ClientMsg::Interrupt]
        .iter()
        .flat_map(encode)
        .collect();

    let mut buf: Vec<u8> = Vec::new();
    let mut seen = Vec::new();
    for chunk in wire.chunks(3) {
        buf.extend_from_slice(chunk);
        while let Some(n) = line_len(&buf) {
            let line = buf.drain(..n).collect::<Vec<u8>>();
            match decode::<ClientMsg>(&line).expect("valid stream") {
                Decoded::Msg(m) => seen.push(m),
                Decoded::Partial => panic!("line_len guaranteed a full line"),
            }
        }
    }
    assert_eq!(seen, vec![ClientMsg::Attach { id: 9 }, ClientMsg::Interrupt]);
}

// ---- handshake waits ------------------------------------------------------
//
// The socket's read timeout is a poll interval, never a deadline. These pin
// the difference: before the fix a reply slower than one poll surfaced as a
// bare `EAGAIN` and killed the front end before it painted anything (a
// 32 000-entry `attach` died at 15 s).

/// A `ClientConn` over an in-process socket pair with a short poll interval,
/// so the tests do not wait 15 s for the real one.
fn pair(poll: std::time::Duration) -> (ClientConn, std::os::unix::net::UnixStream) {
    let (client, server) = std::os::unix::net::UnixStream::pair().unwrap();
    client.set_read_timeout(Some(poll)).unwrap();
    (
        ClientConn {
            commands: Vec::new(),
            stream: std::sync::Arc::new(client),
            buf: Vec::new(),
            scan: LineScanner::default(),
        },
        server,
    )
}

#[test]
fn a_slow_reply_is_waited_for_not_treated_as_a_hangup() {
    let (mut conn, mut server) = pair(std::time::Duration::from_millis(50));
    let writer = std::thread::spawn(move || {
        use std::io::Write;
        // Several poll intervals late: "slow", not "dead".
        std::thread::sleep(std::time::Duration::from_millis(180));
        server
            .write_all(&encode(&ServerMsg::Attached { session_id: 9 }))
            .unwrap();
        server.flush().unwrap();
    });
    let got = conn
        .wait_for_within(
            |m| matches!(m, ServerMsg::Attached { .. }),
            std::time::Duration::from_secs(5),
        )
        .expect("a late reply must be waited for");
    assert_eq!(got, ServerMsg::Attached { session_id: 9 });
    writer.join().unwrap();
}

#[test]
fn a_silent_daemon_names_the_wait_it_gave_up_on() {
    let (mut conn, _server) = pair(std::time::Duration::from_millis(50));
    let err = conn
        .wait_for_within(|_| true, std::time::Duration::from_millis(200))
        .expect_err("no reply must eventually give up");
    assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
    assert_eq!(err.to_string(), "daemon 在 0.2 秒内没有回应");
}

#[test]
fn waiting_drops_messages_the_caller_did_not_ask_for() {
    // The attach flow reads past whatever the daemon pushed first. The gap
    // between the two messages spans several poll intervals on purpose: the
    // wait must retry without losing the message it already read.
    let (mut conn, mut server) = pair(std::time::Duration::from_millis(50));
    let writer = std::thread::spawn(move || {
        use std::io::Write;
        server
            .write_all(&encode(&ServerMsg::HelloOk {
            proto: PROTO_VERSION,
            commands: crate::server::wire::command_table(),
        }))
            .unwrap();
        server.flush().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(180));
        server
            .write_all(&encode(&ServerMsg::Attached { session_id: 4 }))
            .unwrap();
        server.flush().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
    });
    let got = conn
        .wait_for_within(
            |m| matches!(m, ServerMsg::Attached { .. }),
            std::time::Duration::from_secs(5),
        )
        .unwrap();
    assert_eq!(got, ServerMsg::Attached { session_id: 4 });
    writer.join().unwrap();
}

// ---- incremental line scanning --------------------------------------------
//
// `line_len` rescans from byte 0, which made receiving ONE big line
// quadratic. A 32 000-entry transcript snapshot is a single ~39 MB line
// arriving in 16 KiB chunks, so the front end spent tens of seconds
// re-walking it before painting anything.

#[test]
fn one_big_line_arrives_in_chunks() {
    // Functional pin: a line far larger than one read chunk still decodes.
    let (mut conn, mut server) = pair(std::time::Duration::from_millis(50));
    let big = "x".repeat(400_000);
    let bytes = encode(&ServerMsg::Transcript {
        blocks: vec![WireBlock {
            id: 1,
            entries: vec![Entry::User {
                content: big.clone(),
            }],
        }],
        live: Vec::new(),
    });
    assert!(bytes.len() > 400_000);
    let writer = std::thread::spawn(move || {
        use std::io::Write;
        server.write_all(&bytes).unwrap();
        server.flush().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
    });
    let got = conn
        .wait_for_within(|_| true, std::time::Duration::from_secs(10))
        .unwrap();
    match got {
        ServerMsg::Transcript { blocks, .. } => {
            assert_eq!(blocks.len(), 1);
            assert_eq!(blocks[0].entries, vec![Entry::User { content: big }]);
        }
        other => panic!("expected the transcript, got {other:?}"),
    }
    writer.join().unwrap();
}

#[test]
fn scanning_a_growing_buffer_stays_linear() {
    // 40 MB arriving in 16 KiB chunks: the shape of a large snapshot. The
    // naive "rescan from zero" loop re-walks tens of GB here and takes tens
    // of seconds; incremental scanning is milliseconds. The bound is ~200x
    // the measured cost, so it only trips on a real regression.
    let mut buf: Vec<u8> = Vec::new();
    let mut scan = LineScanner::default();
    let chunk = vec![b'x'; 16 * 1024];
    let started = std::time::Instant::now();
    while buf.len() < 40 * 1024 * 1024 {
        buf.extend_from_slice(&chunk);
        assert!(scan.next(&buf).is_none(), "no newline in the payload");
    }
    buf.push(b'\n');
    assert_eq!(scan.next(&buf), Some(buf.len()));
    let elapsed = started.elapsed();
    assert!(
        elapsed < std::time::Duration::from_secs(2),
        "scanning 40 MB in 16 KiB chunks took {elapsed:?}"
    );
}
