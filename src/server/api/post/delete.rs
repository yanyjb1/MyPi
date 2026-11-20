// `delete` — a child is refused; a parent takes its children with it.

use anyhow::Result;
use serde_json::{Value, json};

use crate::ipc::contract::Text;
use crate::server::api::route::i64_of;
use crate::server::api::{Api, Refuse};
use crate::server::storage::Reader;
use crate::server::storage::Table;
use crate::server::storage::query::{Cond, Order, Query, Sheet};

use super::emit;

// The rows this write removed; the api closes their streams once they are gone.
pub(crate) fn apply(api: &Api, viewer: &str, ask: &Value) -> Result<Vec<i64>, Refuse> {
    let session = i64_of(ask, "session")?;
    emit::require_writer(api, viewer, session)?;
    if api.relay().busy(session) {
        api.relay().notice(session, Text::warning("delete_busy"));
        return Err(Refuse::new("delete_busy"));
    }

    let reader = Reader::open(api.db())?;
    let row = reader
        .session(session)?
        .ok_or_else(|| Refuse::new("no_session").args(json!({"session": session})))?;
    if row["anchor"].is_i64() {
        return Err(Refuse::new("child_delete"));
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
    // The memos leave the database before the rows that hold them, one file per session.
    let folder = api
        .cfg()
        .memo_folder()
        .or_else(crate::server::platform::env::downloads_dir);
    if let Some(folder) = folder {
        for id in &ids {
            api.writer()
                .export_memos(*id, &folder)
                .map_err(|_| Refuse::new("internal"))?;
        }
    }
    api.writer().delete_rows(Table::Sessions, &ids)?;
    for id in &ids {
        api.ledger().drop(*id);
    }
    Ok(ids)
}
