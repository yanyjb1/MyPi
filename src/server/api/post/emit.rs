//! What every write door shares: the writer gate and the change event.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::call::store;
use crate::server::api::convert;
use crate::server::api::{Api, Refuse};
use crate::server::storage::Reader;

/// Only the session's writer may change it.
pub(crate) fn require_writer(api: &Api, viewer: &str, session: i64) -> Result<(), Refuse> {
    match api.relay().writer_of(session) {
        Some(who) if who == viewer => Ok(()),
        _ => Err(Refuse::Permission(
            "only the session's writer may write".into(),
        )),
    }
}

/// Announce a session-row change to everyone but the writer.
pub(crate) fn changed(api: &Api, viewer: &str, session: i64, action: &str) -> Result<(), Refuse> {
    let conn = Reader::open(api.db())?;
    let row = store::session_row(&conn, session);
    Reader::close(conn);
    let meta = convert::session_meta(row?);
    api.relay()
        .post(session, action, viewer, json!({"meta": meta}));
    Ok(())
}

/// The session's current working directory, as stored.
pub(crate) fn stored_cwd(api: &Api, session: i64) -> Result<Option<String>, Refuse> {
    let conn = Reader::open(api.db())?;
    let cwd: Option<String> = conn
        .query_row("SELECT cwd FROM sessions WHERE id = ?1", [session], |r| {
            r.get(0)
        })
        .ok()
        .flatten();
    Reader::close(conn);
    Ok(cwd)
}

/// The newest round in the session's list.
pub(crate) fn tip_turn(api: &Api, session: i64) -> Result<Option<i64>, Refuse> {
    let conn = Reader::open(api.db())?;
    let turn: Option<i64> = conn
        .query_row(
            "SELECT turn_id FROM session_turns WHERE session_id = ?1
              ORDER BY idx DESC LIMIT 1",
            [session],
            |r| r.get(0),
        )
        .ok()
        .flatten();
    Reader::close(conn);
    Ok(turn)
}

/// A model pair from `{"provider", "model"}`.
pub(crate) fn model_of(value: &Value) -> Result<(String, String), Refuse> {
    let provider = value
        .get("provider")
        .and_then(Value::as_str)
        .ok_or_else(|| Refuse::Bad("missing or bad `model.provider`".into()))?;
    let model = value
        .get("model")
        .and_then(Value::as_str)
        .ok_or_else(|| Refuse::Bad("missing or bad `model.model`".into()))?;
    Ok((provider.to_string(), model.to_string()))
}
