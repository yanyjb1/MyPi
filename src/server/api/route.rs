// The name tables: kind and name pick the write or read, JSON picks the arguments.

use anyhow::Result;
use serde_json::{Value, json};

use crate::ipc::contract::Frame;

use super::call;
use super::post;
use super::relay::Signal;
use super::subscribe;
use super::{Api, Refuse};

// What one write hands back: the reply's body, what the session stream should hear, and the sessions to close once that has gone out.
pub(crate) struct Reply {
    pub done: Value,
    pub signal: Option<Signal>,
    pub closed: Vec<i64>,
}

impl Reply {
    fn plain() -> Self {
        Self::done(json!({}))
    }

    fn done(value: Value) -> Self {
        Self {
            done: value,
            signal: None,
            closed: Vec::new(),
        }
    }

    fn signal(signal: Signal) -> Self {
        Self {
            done: json!({}),
            signal: Some(signal),
            closed: Vec::new(),
        }
    }
}

pub(crate) fn call(api: &Api, _viewer: &str, frame: &Frame) -> Result<Value, Refuse> {
    let ask = &frame.payload;
    match frame.name.as_str() {
        "init" => call::init::answer(api, i64_of(ask, "session")?),
        "blocks" => call::blocks::answer(api, i64_of(ask, "session")?, ranges_of(ask, "ranges")?),
        "meta" => call::meta::answer(api, i64_of(ask, "session")?),
        "usage" => call::usage::answer(api, i64_of(ask, "session")?),
        "todo" => call::todo::answer(api, i64_of(ask, "session")?),
        "memo" => call::memo::answer(
            api,
            opt_i64_of(ask, "session")?,
            str_of(ask, "op")?,
            opt_str_of(ask, "scope")?,
            opt_str_of(ask, "name")?,
        ),
        "children" => call::children::answer(api, i64_of(ask, "session")?),
        "config" => call::config::answer(api, str_of(ask, "key")?),
        "pools" => call::pools::answer(api, ask),
        "sessions" => call::sessions::answer(api, opt_str_of(ask, "under")?),
        other => Err(Refuse::new("unknown_name").args(json!({"name": other}))),
    }
}

pub(crate) fn post(api: &Api, conn: u64, viewer: &str, frame: &Frame) -> Result<Reply, Refuse> {
    let ask = &frame.payload;
    match frame.name.as_str() {
        "meta" => meta(api, viewer, ask),
        "init" => post::init::apply(api, ask).map(Reply::done),
        "delete" => {
            let ids = post::delete::apply(api, viewer, ask)?;
            let mut reply = Reply::plain();
            reply.closed = ids;
            Ok(reply)
        }
        "memo" => post::memo::apply(api, ask).map(|()| Reply::plain()),
        "default_model" => post::default_model::apply(api, ask).map(|()| Reply::plain()),
        "interrupt" => post::interrupt::apply(api, viewer, ask).map(|()| Reply::plain()),
        "mailbox" => subscribe::mail::apply(api, conn, viewer, ask).map(|()| Reply::plain()),
        other => Err(Refuse::new("unknown_name").args(json!({"name": other}))),
    }
}

// One meta request: its `name` picks the write, its `content` holds that write's arguments.
fn meta(api: &Api, viewer: &str, ask: &Value) -> Result<Reply, Refuse> {
    let name = str_of(ask, "name")?;
    let content = ask
        .get("content")
        .ok_or_else(|| Refuse::new("payload_key_missing").args(json!({"key": "content"})))?;
    let session = i64_of(content, "session")?;
    let (signal, value) = match name {
        "name" => ("name", post::name::apply(api, viewer, content)?),
        "cwd" => ("cwd", post::cwd::apply(api, viewer, content)?),
        "switch" => ("model", post::switch::apply(api, viewer, content)?),
        "effort" => ("effort", post::effort::apply(api, viewer, content)?),
        "rewind" => ("rewind", post::rewind::apply(api, viewer, content)?),
        other => return Err(Refuse::new("unknown_meta").args(json!({"name": other}))),
    };
    Ok(Reply::signal(Signal {
        session,
        name: signal,
        content: value,
    }))
}

pub(crate) fn subscribe(
    api: &Api,
    conn: u64,
    viewer: &str,
    frame: &Frame,
) -> Result<Option<Value>, Refuse> {
    let ask = &frame.payload;
    match frame.name.as_str() {
        "watch" => subscribe::watch::apply(api, conn, opt_i64_of(ask, "session")?).map(|()| None),
        "submit" => subscribe::submit::apply(api, conn, viewer, ask).map(Some),
        "compact" => subscribe::compact::apply(api, conn, viewer, ask).map(Some),
        other => Err(Refuse::new("unknown_name").args(json!({"name": other}))),
    }
}

pub(crate) fn i64_of(value: &Value, key: &str) -> Result<i64, Refuse> {
    value
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| Refuse::new("payload_key_missing").args(json!({"key": key})))
}

pub(crate) fn opt_i64_of(value: &Value, key: &str) -> Result<Option<i64>, Refuse> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_i64()
            .map(Some)
            .ok_or_else(|| Refuse::new("payload_key_bad").args(json!({"key": key}))),
    }
}

pub(crate) fn str_of<'a>(value: &'a Value, key: &str) -> Result<&'a str, Refuse> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Refuse::new("payload_key_missing").args(json!({"key": key})))
}

pub(crate) fn opt_str_of<'a>(value: &'a Value, key: &str) -> Result<Option<&'a str>, Refuse> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_str()
            .map(Some)
            .ok_or_else(|| Refuse::new("payload_key_bad").args(json!({"key": key}))),
    }
}

pub(crate) fn ranges_of(value: &Value, key: &str) -> Result<Vec<[i64; 2]>, Refuse> {
    let items = value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| Refuse::new("payload_key_missing").args(json!({"key": key})))?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let pair = item
            .as_array()
            .filter(|p| p.len() == 2)
            .ok_or_else(|| Refuse::new("payload_key_non_pair").args(json!({"key": key})))?;
        let lo = pair[0]
            .as_i64()
            .ok_or_else(|| Refuse::new("payload_key_non_number").args(json!({"key": key})))?;
        let hi = pair[1]
            .as_i64()
            .ok_or_else(|| Refuse::new("payload_key_non_number").args(json!({"key": key})))?;
        out.push([lo, hi]);
    }
    Ok(out)
}
