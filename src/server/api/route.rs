//! The name tables: kind and name pick the door, JSON picks the arguments.

use anyhow::Result;
use serde_json::Value;

use crate::ipc::contract::Frame;

use super::call;
use super::post;
use super::subscribe;
use super::{Api, Refuse};

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
            str_of(ask, "scope")?,
            opt_str_of(ask, "name")?,
        ),
        "children" => call::children::answer(api, i64_of(ask, "session")?),
        "config" => call::config::answer(api, str_of(ask, "key")?),
        "pools" => call::pools::answer(api),
        "sessions" => call::sessions::answer(api, opt_str_of(ask, "under")?),
        other => Err(Refuse::Internal(format!("no such name: {other}"))),
    }
}

pub(crate) fn post(api: &Api, conn: u64, viewer: &str, frame: &Frame) -> Result<Value, Refuse> {
    let ask = &frame.payload;
    match frame.name.as_str() {
        "init" => post::init::apply(api, conn, viewer, ask),
        "name" => post::name::apply(api, viewer, ask),
        "cwd" => post::cwd::apply(api, viewer, ask),
        "switch" => post::switch::apply(api, viewer, ask),
        "profile" => post::profile::apply(api, viewer, ask),
        "todo_on" => post::todo_on::apply(api, viewer, ask),
        "todo_off" => post::todo_off::apply(api, viewer, ask),
        "delete" => post::delete::apply(api, viewer, ask),
        "interrupt" => post::interrupt::apply(api, viewer, ask),
        "rewind" => post::rewind::apply(api, viewer, ask),
        "default_model" => post::default_model::apply(api, ask),
        "reload" => post::reload::apply(api),
        other => Err(Refuse::Internal(format!("no such name: {other}"))),
    }
}

pub(crate) fn subscribe(
    api: &Api,
    conn: u64,
    viewer: &str,
    frame: &Frame,
) -> Result<Option<Value>, Refuse> {
    let ask = &frame.payload;
    match frame.name.as_str() {
        "watch" => subscribe::watch::apply(api, conn, i64_of(ask, "session")?).map(|_| None),
        "submit" => subscribe::submit::apply(api, conn, viewer, ask).map(Some),
        "compact" => subscribe::compact::apply(api, conn, viewer, ask).map(Some),
        other => Err(Refuse::Internal(format!("no such name: {other}"))),
    }
}

pub(crate) fn i64_of(value: &Value, key: &str) -> Result<i64, Refuse> {
    value
        .get(key)
        .and_then(Value::as_i64)
        .ok_or_else(|| Refuse::Bad(format!("missing or bad `{key}`")))
}

pub(crate) fn opt_i64_of(value: &Value, key: &str) -> Result<Option<i64>, Refuse> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_i64()
            .map(Some)
            .ok_or_else(|| Refuse::Bad(format!("bad `{key}`"))),
    }
}

pub(crate) fn str_of<'a>(value: &'a Value, key: &str) -> Result<&'a str, Refuse> {
    value
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| Refuse::Bad(format!("missing or bad `{key}`")))
}

pub(crate) fn opt_str_of<'a>(value: &'a Value, key: &str) -> Result<Option<&'a str>, Refuse> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v
            .as_str()
            .map(Some)
            .ok_or_else(|| Refuse::Bad(format!("bad `{key}`"))),
    }
}

pub(crate) fn ranges_of(value: &Value, key: &str) -> Result<Vec<[i64; 2]>, Refuse> {
    let items = value
        .get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| Refuse::Bad(format!("missing or bad `{key}`")))?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let pair = item
            .as_array()
            .filter(|p| p.len() == 2)
            .ok_or_else(|| Refuse::Bad(format!("`{key}` holds a non-pair")))?;
        let lo = pair[0]
            .as_i64()
            .ok_or_else(|| Refuse::Bad(format!("`{key}` holds a non-number")))?;
        let hi = pair[1]
            .as_i64()
            .ok_or_else(|| Refuse::Bad(format!("`{key}` holds a non-number")))?;
        out.push([lo, hi]);
    }
    Ok(out)
}
