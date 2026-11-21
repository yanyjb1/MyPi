// `children` — the sessions one conversation spawned, found by the blocks they hang on.

use anyhow::Result;
use serde_json::{Value, json};

use super::sessions;
use crate::server::api::{Api, Refuse};
use crate::server::storage::Reader;
use crate::server::storage::query::{Cond, Order, Query, Sheet};

pub(crate) fn answer(api: &Api, session: i64) -> Result<Value, Refuse> {
    let reader = Reader::open(api.db())?;
    let rows = reader.query(
        &Query::new(Sheet::Sessions)
            .columns(&["id", "name", "cwd", "anchor"])
            .filter(vec![Cond::Within("anchor", reader.ranges(session)?)])
            .order("id", Order::Asc),
    )?;
    let mut out = Vec::with_capacity(rows.len());
    for row in rows {
        let Some(id) = row["id"].as_i64() else {
            continue;
        };
        let mut row = row;
        if let Value::Object(map) = &mut row {
            let stats = sessions::stats(&reader, id)?;
            map.insert("rounds".to_string(), json!(stats.rounds));
            map.insert("bytes".to_string(), json!(stats.bytes));
            map.insert("last_ts".to_string(), json!(stats.last_ts));
            map.insert("first_user".to_string(), json!(stats.first_user));
            // Whether the loop holds this session right now: a child is unbooked the moment its task ends.
            map.insert("running".to_string(), json!(api.ledger().booked(id)));
        }
        out.push(row);
    }
    Ok(json!({"session": session, "rows": out}))
}
