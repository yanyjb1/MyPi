//! `sessions` — the resume list: main sessions only, newest first, plus what a row draws.

use std::collections::HashSet;

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::{Api, Refuse};
use crate::server::storage::Reader;
use crate::server::storage::query::{Cond, Order, Query, Sheet, Val};

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
        rows.push(extras(&reader, row)?);
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

/// Besides the session's own columns, a row carries what the picker draws: the first user
/// message, the transcript's size, the time of the last block, and how many memos the session
/// holds.
fn extras(reader: &Reader, row: Value) -> Result<Value, Refuse> {
    let Some(session) = row.get("id").and_then(Value::as_i64) else {
        return Ok(row);
    };
    let ranges = reader.ranges(session)?;
    let mut first_user: Option<String> = None;
    let mut bytes: i64 = 0;
    let mut last_ts: Option<String> = None;
    let mut memos: i64 = 0;

    if !ranges.is_empty() {
        let blocks = reader.query(
            &Query::new(Sheet::Blocks)
                .columns(&["kind", "content", "text", "result", "ts"])
                .filter(vec![Cond::Within("id", ranges.clone())])
                .order("id", Order::Asc),
        )?;
        for block in &blocks {
            if first_user.is_none() && block["kind"].as_str() == Some("user") {
                first_user = block["content"]
                    .as_str()
                    .or_else(|| block["text"].as_str())
                    .map(str::to_string);
            }
            for key in ["content", "text", "result"] {
                if let Some(text) = block[key].as_str() {
                    bytes += text.len() as i64;
                }
            }
            if let Some(ts) = block["ts"].as_str() {
                last_ts = Some(ts.to_string());
            }
        }

        let named = reader.query(
            &Query::new(Sheet::Memos)
                .columns(&["name"])
                .filter(vec![Cond::Within("anchor", ranges)]),
        )?;
        let mut names: HashSet<&str> = HashSet::new();
        for memo in &named {
            if let Some(name) = memo["name"].as_str() {
                names.insert(name);
            }
        }
        memos = names.len() as i64;
    }

    match row {
        Value::Object(mut map) => {
            map.insert("first_user".to_string(), json!(first_user));
            map.insert("bytes".to_string(), json!(bytes));
            map.insert("last_ts".to_string(), json!(last_ts));
            map.insert("memos".to_string(), json!(memos));
            Ok(Value::Object(map))
        }
        other => Ok(other),
    }
}
