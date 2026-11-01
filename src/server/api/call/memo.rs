//! `memo` — names, one memo's content, and its versions.

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use crate::server::api::{Api, Refuse};
use crate::server::storage::Reader;
use crate::server::storage::query::{Cond, Order, Query, Sheet, Val};

pub(crate) fn answer(
    api: &Api,
    session: Option<i64>,
    op: &str,
    scope: &str,
    name: Option<&str>,
) -> Result<Value, Refuse> {
    let reader = Reader::open(api.db())?;
    let mut filter = match scope {
        "global" => vec![Cond::Null("anchor")],
        "current" => {
            let session = required(session)?;
            vec![Cond::Within("anchor", reader.ranges(session)?)]
        }
        other => return Err(Refuse::Bad(format!("no such scope: {other}"))),
    };
    if let Some(name) = name {
        filter.push(Cond::Eq("name", Val::Text(name.to_string())));
    }
    let rows = reader.query(
        &Query::new(Sheet::Memos)
            .columns(&["id", "anchor", "name", "content", "ts"])
            .filter(filter)
            .order("id", Order::Asc),
    )?;

    match op {
        "list" => {
            let mut names: Vec<&str> = rows.iter().filter_map(|row| row["name"].as_str()).collect();
            names.sort_unstable();
            names.dedup();
            Ok(json!({"names": names}))
        }
        "show" => {
            let row = rows
                .last()
                .ok_or_else(|| anyhow!("no memo with that name"))?;
            Ok(json!({"content": row["content"]}))
        }
        "history" => {
            if rows.is_empty() {
                return Err(Refuse::from(anyhow!("no memo with that name")));
            }
            let versions: Vec<Value> = rows
                .iter()
                .map(|row| json!({"id": row["id"], "content": row["content"], "ts": row["ts"]}))
                .collect();
            Ok(json!({"history": versions}))
        }
        other => Err(Refuse::Bad(format!("no such memo op: {other}"))),
    }
}

fn required(session: Option<i64>) -> Result<i64, Refuse> {
    session.ok_or_else(|| Refuse::Bad("a session is required for the current scope".into()))
}
