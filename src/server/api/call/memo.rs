//! `memo` — every name in both scopes with its latest text, and one name's versions.

use std::collections::BTreeMap;

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::{Api, Refuse};
use crate::server::storage::Reader;
use crate::server::storage::query::{Cond, Order, Query, Sheet, Val};

/// The scopes a list reaches, in the order the answer carries them.
const SCOPES: [&str; 2] = ["current", "global"];

pub(crate) fn answer(
    api: &Api,
    session: Option<i64>,
    op: &str,
    scope: Option<&str>,
    name: Option<&str>,
) -> Result<Value, Refuse> {
    let reader = Reader::open(api.db())?;
    match op {
        "list" => Ok(json!({"memos": list(&reader, session)?})),
        "history" => {
            let scope = scope.ok_or_else(|| Refuse::Bad("history needs a scope".into()))?;
            let name = name.ok_or_else(|| Refuse::Bad("history needs a name".into()))?;
            Ok(json!({
                "scope": scope,
                "name": name,
                "versions": history(&reader, session, scope, name)?,
            }))
        }
        other => Err(Refuse::Bad(format!("no such memo op: {other}"))),
    }
}

/// One row per name and scope, carrying the newest version's content.
fn list(reader: &Reader, session: Option<i64>) -> Result<Vec<Value>, Refuse> {
    let mut out: Vec<Value> = Vec::new();
    for scope in SCOPES {
        let Some(filter) = filter(reader, session, scope)? else {
            continue;
        };
        let rows = reader.query(
            &Query::new(Sheet::Memos)
                .columns(&["id", "name", "content"])
                .filter(filter)
                .order("id", Order::Asc),
        )?;
        let mut latest: BTreeMap<String, String> = BTreeMap::new();
        for row in rows {
            let (Some(name), Some(content)) = (row["name"].as_str(), row["content"].as_str())
            else {
                continue;
            };
            latest.insert(name.to_string(), content.to_string());
        }
        for (name, content) in latest {
            out.push(json!({"scope": scope, "name": name, "content": content}));
        }
    }
    out.sort_by(|a, b| rank(a).cmp(&rank(b)));
    Ok(out)
}

/// Name first, scope second, so one name's two scopes sit together.
fn rank(row: &Value) -> (&str, &str) {
    (
        row["name"].as_str().unwrap_or_default(),
        row["scope"].as_str().unwrap_or_default(),
    )
}

/// Every version of one name, oldest first.
fn history(
    reader: &Reader,
    session: Option<i64>,
    scope: &str,
    name: &str,
) -> Result<Vec<Value>, Refuse> {
    let Some(mut filter) = filter(reader, session, scope)? else {
        return Ok(Vec::new());
    };
    filter.push(Cond::Eq("name", Val::Text(name.to_string())));
    Ok(reader.query(
        &Query::new(Sheet::Memos)
            .columns(&["id", "ts", "content"])
            .filter(filter)
            .order("id", Order::Asc),
    )?)
}

/// One scope's conditions; `None` = this scope holds nothing here.
fn filter(reader: &Reader, session: Option<i64>, scope: &str) -> Result<Option<Vec<Cond>>, Refuse> {
    Ok(match scope {
        "global" => Some(vec![Cond::Null("anchor")]),
        "current" => match session {
            Some(session) => {
                let ranges = reader.ranges(session)?;
                (!ranges.is_empty()).then_some(vec![Cond::Within("anchor", ranges)])
            }
            None => None,
        },
        other => return Err(Refuse::Bad(format!("no such scope: {other}"))),
    })
}
