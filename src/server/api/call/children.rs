//! `children` — the sessions a `task` call spawned, found by the block they hang on.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::{Api, Refuse};
use crate::server::storage::Reader;
use crate::server::storage::query::{Cond, Order, Query, Sheet};

pub(crate) fn answer(api: &Api, session: i64) -> Result<Value, Refuse> {
    let reader = Reader::open(api.db())?;
    let rows = reader.query(
        &Query::new(Sheet::Sessions)
            .columns(&["id", "anchor"])
            .filter(vec![Cond::Within("anchor", reader.ranges(session)?)])
            .order("id", Order::Asc),
    )?;
    Ok(json!({"rows": rows}))
}
