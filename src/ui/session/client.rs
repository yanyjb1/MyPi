//! The wire: one connection, one queue of door calls, one table of answers still owed.

use std::collections::VecDeque;

use anyhow::{Context, Result};
use serde_json::{Value, json};

use crate::ipc::client::Client as Socket;
use crate::ipc::contract::{ErrorCode, ErrorPayload, Frame};
use crate::ipc::endpoint::Endpoint;

use super::event::{
    Change, MemoOp, MemoScope, ModelId, NoticeLevel, Piece, Retention, SessionMeta,
};

/// The configuration key that sizes the raw content cache.
pub const BUDGET_KEY: &str = "config.ui.session.preloadCache";

/// One queued request: the session's translation of a front end intent.
#[derive(Debug)]
pub enum Request {
    Call(Call),
    Post(Post),
    Subscribe(Subscribe),
    Bye,
}

/// The read doors.
#[derive(Debug)]
pub enum Call {
    Init {
        session: i64,
    },
    Blocks {
        session: i64,
        ranges: Vec<[i64; 2]>,
    },
    Meta {
        session: i64,
    },
    Usage {
        session: i64,
    },
    Todo {
        session: i64,
    },
    Memo {
        session: Option<i64>,
        op: MemoOp,
        scope: MemoScope,
        name: Option<String>,
    },
    Children {
        session: i64,
    },
    Config {
        key: String,
    },
    Pools,
    Sessions {
        under: Option<String>,
    },
    /// The cache's budget, asked once by the session itself.
    Budget,
}

/// The write doors.
#[derive(Debug)]
pub enum Post {
    Init {
        cwd: String,
        name: Option<String>,
        model: Option<ModelId>,
        profile: Option<String>,
    },
    Name {
        session: i64,
        name: Option<String>,
    },
    Cwd {
        session: i64,
        path: String,
    },
    Switch {
        session: i64,
        model: ModelId,
    },
    Delete {
        session: i64,
    },
    Interrupt {
        session: i64,
    },
    Rewind {
        session: i64,
        turn: i64,
        block: i64,
    },
    DefaultModel {
        model: ModelId,
    },
}

/// The three subscriptions; `watch` and `bye` expect no answer.
#[derive(Debug)]
pub enum Subscribe {
    Watch {
        session: i64,
    },
    Submit {
        session: i64,
        prefix: Option<String>,
        text: String,
        suffix: Option<String>,
    },
    Compact {
        session: i64,
        retention: Option<Retention>,
        focus: Option<String>,
    },
}

/// What came back: an answer, a refusal, a local failure, or a push.
#[derive(Debug)]
pub enum Arrival {
    Answer {
        request: Request,
        payload: Value,
    },
    Refused {
        request: Request,
        code: ErrorCode,
        message: String,
    },
    Lost {
        request: Request,
        message: String,
    },
    Push(Push),
}

/// One unsolicited frame, classified by its shape.
#[derive(Debug)]
pub enum Push {
    Piece {
        session: i64,
        piece: Piece,
    },
    Structure {
        session: i64,
        turn: i64,
        ranges: Vec<[i64; 2]>,
    },
    Notice {
        session: i64,
        level: NoticeLevel,
        text: String,
    },
    Change {
        session: i64,
        change: Change,
    },
}

/// The connection, the queue, and every request already sent.
pub struct Client {
    socket: Socket,
    queue: VecDeque<Request>,
    waiting: VecDeque<Request>,
}

impl Client {
    /// Connect and register. Who starts the server is not this layer's business.
    pub fn connect(endpoint: Endpoint, viewer: Value) -> Result<Client> {
        let socket = Socket::connect(&endpoint, &viewer)
            .with_context(|| format!("cannot reach the server at {endpoint:?}"))?;
        Ok(Client {
            socket,
            queue: VecDeque::new(),
            waiting: VecDeque::new(),
        })
    }

    pub fn queue(&mut self, request: Request) {
        self.queue.push_back(request);
    }

    /// Drop everything not yet sent.
    pub fn clear(&mut self) {
        self.queue.clear();
    }

    /// Take what arrived, send what is queued, take what that earned.
    pub fn pump(&mut self) -> Vec<Arrival> {
        let mut out = Vec::new();
        self.receive(&mut out);
        while let Some(request) = self.queue.pop_front() {
            let frame = request.frame();
            match self.socket.send(frame) {
                Ok(()) => self.waiting.push_back(request),
                Err(e) => out.push(Arrival::Lost {
                    request,
                    message: format!("{e:#}"),
                }),
            }
        }
        self.receive(&mut out);
        out
    }

    fn receive(&mut self, out: &mut Vec<Arrival>) {
        for frame in self.socket.drain() {
            if frame.kind == "stream" {
                if let Some(push) = decode_push(frame) {
                    out.push(Arrival::Push(push));
                }
                continue;
            }
            let label = (frame.kind.as_str(), frame.name.as_str());
            loop {
                let Some(head) = self.waiting.front() else {
                    break;
                };
                let matched = head.kind() == label.0 && head.name() == label.1;
                let answers = head.replies();
                let request = self.waiting.pop_front().expect("the head was just read");
                if answers && matched {
                    out.push(answer(request, frame.payload));
                    break;
                }
                if answers {
                    out.push(Arrival::Lost {
                        request,
                        message: format!("unexpected reply: {} {}", label.0, label.1),
                    });
                    break;
                }
                if matched {
                    out.push(refused(request, frame.payload));
                    break;
                }
            }
        }
    }
}

impl Request {
    fn kind(&self) -> &'static str {
        match self {
            Request::Call(_) => "call",
            Request::Post(_) => "post",
            Request::Subscribe(_) => "subscribe",
            Request::Bye => "register",
        }
    }

    fn name(&self) -> &'static str {
        match self {
            Request::Call(call) => call.name(),
            Request::Post(post) => post.name(),
            Request::Subscribe(subscribe) => subscribe.name(),
            Request::Bye => "bye",
        }
    }

    /// Does this door answer at all?
    fn replies(&self) -> bool {
        !matches!(
            self,
            Request::Bye | Request::Subscribe(Subscribe::Watch { .. })
        )
    }

    /// The session this request names, if any.
    pub fn session(&self) -> Option<i64> {
        match self {
            Request::Call(call) => call.session(),
            Request::Post(post) => post.session(),
            Request::Subscribe(subscribe) => Some(subscribe.session()),
            Request::Bye => None,
        }
    }

    pub fn fail_kind(&self) -> super::event::FailKind {
        use super::event::FailKind;
        match self {
            Request::Call(_) => FailKind::Read,
            Request::Post(_) => FailKind::Change,
            Request::Subscribe(Subscribe::Watch { .. }) => FailKind::Watch,
            Request::Subscribe(_) => FailKind::Round,
            Request::Bye => FailKind::Change,
        }
    }

    fn frame(&self) -> Frame {
        match self {
            Request::Call(call) => Frame::new("call", call.name(), call.payload()),
            Request::Post(post) => Frame::new("post", post.name(), post.payload()),
            Request::Subscribe(subscribe) => {
                Frame::new("subscribe", subscribe.name(), subscribe.payload())
            }
            Request::Bye => Frame::new("register", "bye", json!({})),
        }
    }
}

impl Call {
    fn name(&self) -> &'static str {
        match self {
            Call::Init { .. } => "init",
            Call::Blocks { .. } => "blocks",
            Call::Meta { .. } => "meta",
            Call::Usage { .. } => "usage",
            Call::Todo { .. } => "todo",
            Call::Memo { .. } => "memo",
            Call::Children { .. } => "children",
            Call::Config { .. } | Call::Budget => "config",
            Call::Pools => "pools",
            Call::Sessions { .. } => "sessions",
        }
    }

    fn payload(&self) -> Value {
        match self {
            Call::Init { session }
            | Call::Meta { session }
            | Call::Usage { session }
            | Call::Todo { session }
            | Call::Children { session } => json!({"session": session}),
            Call::Blocks { session, ranges } => json!({"session": session, "ranges": ranges}),
            Call::Memo {
                session,
                op,
                scope,
                name,
            } => json!({
                "session": session,
                "op": op.as_str(),
                "scope": scope.as_str(),
                "name": name,
            }),
            Call::Config { key } => json!({"key": key}),
            Call::Budget => json!({"key": BUDGET_KEY}),
            Call::Pools => json!({}),
            Call::Sessions { under } => json!({"under": under}),
        }
    }

    fn session(&self) -> Option<i64> {
        match self {
            Call::Init { session }
            | Call::Blocks { session, .. }
            | Call::Meta { session }
            | Call::Usage { session }
            | Call::Todo { session }
            | Call::Children { session } => Some(*session),
            Call::Memo { session, .. } => *session,
            Call::Config { .. } | Call::Budget | Call::Pools | Call::Sessions { .. } => None,
        }
    }
}

impl Post {
    fn name(&self) -> &'static str {
        match self {
            Post::Init { .. } => "init",
            Post::Name { .. } => "name",
            Post::Cwd { .. } => "cwd",
            Post::Switch { .. } => "switch",
            Post::Delete { .. } => "delete",
            Post::Interrupt { .. } => "interrupt",
            Post::Rewind { .. } => "rewind",
            Post::DefaultModel { .. } => "default_model",
        }
    }

    fn payload(&self) -> Value {
        match self {
            Post::Init {
                cwd,
                name,
                model,
                profile,
            } => json!({
                "cwd": cwd,
                "name": name,
                "model": model.as_ref().map(model_json),
                "profile": profile,
            }),
            Post::Name { session, name } => json!({"session": session, "name": name}),
            Post::Cwd { session, path } => json!({"session": session, "path": path}),
            Post::Switch { session, model } => {
                json!({"session": session, "model": model_json(model)})
            }
            Post::Delete { session } | Post::Interrupt { session } => json!({"session": session}),
            Post::Rewind {
                session,
                turn,
                block,
            } => json!({"session": session, "turn": turn, "block": block}),
            Post::DefaultModel { model } => json!({"model": model_json(model)}),
        }
    }

    fn session(&self) -> Option<i64> {
        match self {
            Post::Init { .. } | Post::DefaultModel { .. } => None,
            Post::Name { session, .. }
            | Post::Cwd { session, .. }
            | Post::Switch { session, .. }
            | Post::Delete { session }
            | Post::Interrupt { session }
            | Post::Rewind { session, .. } => Some(*session),
        }
    }
}

impl Subscribe {
    fn name(&self) -> &'static str {
        match self {
            Subscribe::Watch { .. } => "watch",
            Subscribe::Submit { .. } => "submit",
            Subscribe::Compact { .. } => "compact",
        }
    }

    fn payload(&self) -> Value {
        match self {
            Subscribe::Watch { session } => json!({"session": session}),
            Subscribe::Submit {
                session,
                prefix,
                text,
                suffix,
            } => json!({
                "session": session,
                "prefix": prefix,
                "text": text,
                "suffix": suffix,
            }),
            Subscribe::Compact {
                session,
                retention,
                focus,
            } => {
                let mut payload = json!({"session": session, "focus": focus});
                if let Some(retention) = retention {
                    payload["retention"] = retention_json(retention);
                }
                payload
            }
        }
    }

    fn session(&self) -> i64 {
        match self {
            Subscribe::Watch { session }
            | Subscribe::Submit { session, .. }
            | Subscribe::Compact { session, .. } => *session,
        }
    }
}

fn model_json(model: &ModelId) -> Value {
    json!({"provider": model.provider, "model": model.model})
}

fn retention_json(retention: &Retention) -> Value {
    match retention {
        Retention::Chars(count) => json!({"chars": count}),
        Retention::Blocks(count) => json!({"blocks": count}),
    }
}

/// Classify one unsolicited frame by its name and payload.
fn decode_push(frame: Frame) -> Option<Push> {
    let payload = frame.payload;
    let session = payload.get("session")?.as_i64()?;
    if frame.name == "notice" {
        let level = match payload.get("level")?.as_str()? {
            "info" => NoticeLevel::Info,
            "warn" => NoticeLevel::Warn,
            "error" => NoticeLevel::Error,
            _ => return None,
        };
        return Some(Push::Notice {
            session,
            level,
            text: payload.get("content")?.as_str()?.to_string(),
        });
    }
    if frame.name == "change" {
        return Some(Push::Change {
            session,
            change: decode_change(&payload)?,
        });
    }
    if payload.get("ranges").is_some() {
        return Some(Push::Structure {
            session,
            turn: payload.get("turn")?.as_i64()?,
            ranges: ranges_of(payload.get("ranges")?)?,
        });
    }
    Some(Push::Piece {
        session,
        piece: decode_piece(&payload)?,
    })
}

fn decode_change(payload: &Value) -> Option<Change> {
    let action = payload.get("action")?.as_str()?;
    match action {
        "name" | "cwd" | "switch" => {
            let meta: SessionMeta = serde_json::from_value(payload.get("meta")?.clone()).ok()?;
            Some(Change::Meta(meta))
        }
        "rewind" => Some(Change::Rewound {
            turn: payload.get("turn")?.as_i64()?,
            block: payload.get("block")?.as_i64()?,
            cut: serde_json::from_value(payload.get("cut")?.clone()).ok()?,
        }),
        "delete" => Some(Change::Deleted),
        _ => None,
    }
}

fn decode_piece(payload: &Value) -> Option<Piece> {
    let kind = payload.get("kind")?.as_str()?;
    let text = || payload.get("text")?.as_str().map(str::to_string);
    Some(match kind {
        "text" => Piece::Text(text()?),
        "thinking" => Piece::Thinking(text()?),
        "arguments" => Piece::Arguments {
            call_id: text_of(payload, "call_id")?,
            text: text()?,
        },
        "boundary" => Piece::Boundary,
        "tool_start" => Piece::ToolStart {
            call_id: text_of(payload, "call_id")?,
            name: text_of(payload, "name")?,
            args: text()?,
            intent: payload
                .get("intent")
                .and_then(Value::as_str)
                .map(str::to_string),
        },
        "tool_progress" => Piece::ToolProgress {
            call_id: text_of(payload, "call_id")?,
            text: text()?,
        },
        "tool_end" => Piece::ToolEnd {
            call_id: text_of(payload, "call_id")?,
            ok: payload.get("ok")?.as_bool()?,
            result: text()?,
            memo: payload
                .get("memo")
                .and_then(Value::as_str)
                .map(str::to_string),
            duration_ms: payload.get("duration_ms")?.as_u64()?,
        },
        _ => return None,
    })
}

fn text_of(payload: &Value, key: &str) -> Option<String> {
    payload.get(key)?.as_str().map(str::to_string)
}

fn ranges_of(value: &Value) -> Option<Vec<[i64; 2]>> {
    let mut out = Vec::new();
    for item in value.as_array()? {
        let pair = item.as_array()?;
        if pair.len() != 2 {
            return None;
        }
        out.push([pair[0].as_i64()?, pair[1].as_i64()?]);
    }
    Some(out)
}

/// A frame that answers a request: an error payload, or the answer itself.
fn answer(request: Request, payload: Value) -> Arrival {
    match serde_json::from_value::<ErrorPayload>(payload.clone()) {
        Ok(error) => Arrival::Refused {
            request,
            code: error.error.code,
            message: error.error.message,
        },
        Err(_) => Arrival::Answer { request, payload },
    }
}

/// A frame that answers a silent request: only an error can come back.
fn refused(request: Request, payload: Value) -> Arrival {
    match serde_json::from_value::<ErrorPayload>(payload.clone()) {
        Ok(error) => Arrival::Refused {
            request,
            code: error.error.code,
            message: error.error.message,
        },
        Err(_) => Arrival::Lost {
            request,
            message: "a silent request answered with a body".to_string(),
        },
    }
}
