//! `delete` — a child is refused; a parent takes its children with it.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::route::i64_of;
use crate::server::api::{Api, Refuse};
use crate::server::storage::Reader;
use crate::server::storage::Table;
use crate::server::storage::query::{Cond, Order, Query, Sheet};

use super::emit;

pub(crate) fn apply(api: &Api, viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let session = i64_of(ask, "session")?;
    emit::require_writer(api, viewer, session)?;
    if api.relay().busy(session) {
        return Err(Refuse::Busy("the session is busy".into()));
    }

    let reader = Reader::open(api.db())?;
    let row = reader
        .session(session)?
        .ok_or_else(|| Refuse::Bad(format!("no session {session}")))?;
    if row["anchor"].is_i64() {
        return Err(Refuse::Permission(
            "a child session cannot be deleted on its own".into(),
        ));
    }
    let children = reader.query(
        &Query::new(Sheet::Sessions)
            .columns(&["id"])
            .filter(vec![Cond::Within("anchor", reader.ranges(session)?)])
            .order("id", Order::Asc),
    )?;
    drop(reader);

    let mut ids: Vec<i64> = children
        .iter()
        .filter_map(|row| row["id"].as_i64())
        .collect();
    ids.push(session);
    api.relay()
        .change(session, "delete", viewer, json!({"session": session}));
    api.writer().delete_rows(Table::Sessions, &ids)?;
    for id in &ids {
        api.ledger().drop(*id);
        api.relay().forget(*id);
    }
    #[cfg(feature = "web")]
    for id in &ids {
        crate::server::agent::tools::web::close_session(*id);
    }
    Ok(json!({}))
}
