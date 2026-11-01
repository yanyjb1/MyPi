//! `sessions` — the resume list: main sessions only, newest first.

use std::collections::HashSet;

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::{Api, Refuse};
use crate::server::storage::Reader;
use crate::server::storage::query::{Cond, Query, Sheet, Val};

pub(crate) fn answer(api: &Api, under: Option<&str>) -> Result<Value, Refuse> {
    let reader = Reader::open(api.db())?;
    let walked = match under {
        Some(dir) => Some(dirs(&reader, dir)?),
        None => None,
    };
    let mut rows = Vec::new();
    for row in reader.sessions()? {
        if row["anchor"].is_i64() {
            continue;
        }
        if let Some(dir) = under {
            let here = row["cwd"].as_str() == Some(dir);
            let before = match row.get("id").and_then(Value::as_i64) {
                Some(id) => walked.as_ref().is_some_and(|set| set.contains(&id)),
                None => false,
            };
            if !here && !before {
                continue;
            }
        }
        rows.push(row);
    }
    rows.reverse();
    Ok(json!({"rows": rows}))
}

/// The sessions that ever ran a turn in that directory.
fn dirs(reader: &Reader, dir: &str) -> Result<HashSet<i64>> {
    let rows = reader.query(
        &Query::new(Sheet::Cwds)
            .columns(&["session_id"])
            .filter(vec![Cond::Eq("cwd", Val::Text(dir.to_string()))]),
    )?;
    Ok(rows
        .iter()
        .filter_map(|row| row["session_id"].as_i64())
        .collect())
}
