//! `memo` — user notes: add and mod append a version, del removes every version.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::route::{i64_of, opt_str_of, str_of};
use crate::server::api::{Api, Refuse};
use crate::server::storage::query::{Cond, Query, Sheet, Val};
use crate::server::storage::{Reader, Ref, Row, Submit, Table};

/// Where a row hangs: one session, or none.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Scope {
    Current,
    Global,
}

impl Scope {
    fn as_str(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Global => "global",
        }
    }
}

pub(crate) fn apply(api: &Api, _conn: u64, _viewer: &str, ask: &Value) -> Result<Value, Refuse> {
    let op = str_of(ask, "op")?;
    let scope = scope(ask)?;
    let name = name(str_of(ask, "name")?)?;
    match op {
        "add" | "mod" => {
            let content = str_of(ask, "content")?;
            let anchor = anchor(api, scope, ask)?;
            api.writer().push(Submit {
                rows: vec![Row::Memo {
                    anchor: anchor.map(Ref::Known),
                    name: name.clone(),
                    content: content.to_string(),
                    ts: crate::server::platform::time::now_stamp(),
                }],
            })?;
            Ok(json!({"scope": scope.as_str(), "name": name}))
        }
        "del" => {
            let removed = delete(api, scope, ask, &name)?;
            Ok(json!({"scope": scope.as_str(), "name": name, "removed": removed}))
        }
        other => Err(Refuse::Bad(format!("no such memo op: {other}"))),
    }
}

/// A write lands global unless the caller names another scope.
fn scope(ask: &Value) -> Result<Scope, Refuse> {
    match opt_str_of(ask, "scope")?.unwrap_or("global") {
        "global" => Ok(Scope::Global),
        "current" => Ok(Scope::Current),
        other => Err(Refuse::Bad(format!("no such scope: {other}"))),
    }
}

/// A name is one word; the tool's rule, kept here too.
fn name(raw: &str) -> Result<String, Refuse> {
    let name = raw.trim();
    if name.is_empty() || name.chars().any(char::is_whitespace) {
        return Err(Refuse::Bad(
            "a memo name is non-empty and has no spaces".into(),
        ));
    }
    Ok(name.to_string())
}

/// The block a current-scope row hangs on: the session's newest one.
fn anchor(api: &Api, scope: Scope, ask: &Value) -> Result<Option<i64>, Refuse> {
    if scope == Scope::Global {
        return Ok(None);
    }
    let reader = Reader::open(api.db())?;
    let ranges = reader.ranges(i64_of(ask, "session")?)?;
    let Some([_, hi]) = ranges.last() else {
        return Err(Refuse::Bad("this session holds no blocks yet".into()));
    };
    Ok(Some(*hi))
}

/// Every version of that name in that scope goes; `0` = there was nothing to delete.
fn delete(api: &Api, scope: Scope, ask: &Value, name: &str) -> Result<usize, Refuse> {
    let reader = Reader::open(api.db())?;
    let mut filter = match scope {
        Scope::Global => vec![Cond::Null("anchor")],
        Scope::Current => vec![Cond::Within(
            "anchor",
            reader.ranges(i64_of(ask, "session")?)?,
        )],
    };
    filter.push(Cond::Eq("name", Val::Text(name.to_string())));
    let rows = reader.query(&Query::new(Sheet::Memos).columns(&["id"]).filter(filter))?;
    let ids: Vec<i64> = rows.iter().filter_map(|row| row["id"].as_i64()).collect();
    if ids.is_empty() {
        return Ok(0);
    }
    api.writer()
        .delete_rows(Table::Memos, &ids)
        .map_err(Refuse::from)
}
