//! `delete` — a child is refused; a parent takes its children with it.

use anyhow::Result;
use rusqlite::OptionalExtension;
use serde_json::{Value, json};

use crate::server::api::call::store::block_ids;
use crate::server::api::route::i64_of;
use crate::server::api::{Api, Refuse};
use crate::server::storage::Reader;

use super::emit;

pub(crate) fn apply(api: &Api, viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let session = i64_of(ask, "session")?;
    emit::require_writer(api, viewer, session)?;
    if api.relay().running(session) || api.register().running(session) {
        return Err(Refuse::Busy("the session is busy".into()));
    }

    let conn = Reader::open(api.db())?;
    let child: Option<i64> = conn
        .query_row(
            "SELECT subsession_id FROM subsessions WHERE subsession_id = ?1",
            [session],
            |r| r.get(0),
        )
        .optional()?;
    if child.is_some() {
        Reader::close(conn);
        return Err(Refuse::Internal(
            "a child session cannot be deleted on its own".into(),
        ));
    }

    let anchors = block_ids(&conn, session)?;
    let mut stmt = conn.prepare("SELECT subsession_id, anchor FROM subsessions")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
    let mut children = Vec::new();
    for row in rows {
        let (child, anchor) = row?;
        if anchors.contains(&anchor) {
            children.push(child);
        }
    }
    drop(stmt);
    Reader::close(conn);

    for child in children {
        api.writer().delete_session(child)?;
        api.register().drop(child);
    }
    api.writer().delete_session(session)?;
    api.register().drop(session);
    api.relay()
        .post(session, "delete", viewer, json!({"session": session}));
    api.relay().forget(session);
    Ok(json!({}))
}
