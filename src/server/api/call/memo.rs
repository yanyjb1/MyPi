//! `memo` — names, one memo's content, and its versions.

use anyhow::{Result, anyhow};
use rusqlite::Connection;
use serde_json::{Value, json};
use std::collections::BTreeSet;

use crate::server::api::call::store;
use crate::server::api::{Api, Refuse};
use crate::server::storage::Reader;

pub(crate) fn answer(
    api: &Api,
    session: Option<i64>,
    op: &str,
    scope: &str,
    name: Option<&str>,
) -> Result<Value, Refuse> {
    let conn = Reader::open(api.db())?;
    let out = (|| -> Result<Value> {
        match (op, scope) {
            ("list", "current") => Ok(json!({"names": names_current(&conn, session)?})),
            ("list", "global") => Ok(json!({"names": names_global(&conn)?})),
            ("show", "current") => Ok(json!({"content": content_current(&conn, session, name)?})),
            ("show", "global") => Ok(json!({"content": content_global(&conn, name)?})),
            ("history", "current") => {
                Ok(json!({"history": history_current(&conn, session, name)?}))
            }
            ("history", "global") => Ok(json!({"history": history_global(&conn, name)?})),
            (op, _) => Err(anyhow!("unknown memo op: {op}")),
        }
    })();
    Reader::close(conn);
    Ok(out?)
}

fn names_current(conn: &Connection, session: Option<i64>) -> Result<Vec<String>> {
    let session = required(session)?;
    let anchors = store::block_ids(conn, session)?;
    let mut out = BTreeSet::new();
    let mut stmt = conn.prepare("SELECT anchor, name FROM memos WHERE anchor IS NOT NULL")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
    for row in rows {
        let (anchor, name) = row?;
        if anchors.contains(&anchor) {
            out.insert(name);
        }
    }
    Ok(out.into_iter().collect())
}

fn names_global(conn: &Connection) -> Result<Vec<String>> {
    let mut out = BTreeSet::new();
    let mut stmt = conn.prepare("SELECT DISTINCT name FROM memos WHERE anchor IS NULL")?;
    let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
    for row in rows {
        out.insert(row?);
    }
    Ok(out.into_iter().collect())
}

fn content_current(conn: &Connection, session: Option<i64>, name: Option<&str>) -> Result<String> {
    let name = required_name(name)?;
    let session = required(session)?;
    let anchors = store::block_ids(conn, session)?;
    let mut stmt = conn.prepare(
        "SELECT anchor, content FROM memos
          WHERE anchor IS NOT NULL AND name = ?1 ORDER BY id DESC",
    )?;
    let rows = stmt.query_map([name], |r| {
        Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
    })?;
    for row in rows {
        let (anchor, content) = row?;
        if anchors.contains(&anchor) {
            return Ok(content);
        }
    }
    Err(anyhow!("no memo named {name} in this session"))
}

fn content_global(conn: &Connection, name: Option<&str>) -> Result<String> {
    let name = required_name(name)?;
    conn.query_row(
        "SELECT content FROM memos
          WHERE anchor IS NULL AND name = ?1 ORDER BY id DESC LIMIT 1",
        [name],
        |r| r.get(0),
    )
    .map_err(|_| anyhow!("no global memo with that name"))
}

fn history_current(
    conn: &Connection,
    session: Option<i64>,
    name: Option<&str>,
) -> Result<Vec<Value>> {
    let name = required_name(name)?;
    let session = required(session)?;
    let anchors = store::block_ids(conn, session)?;
    let mut stmt = conn.prepare(
        "SELECT id, anchor, content, ts FROM memos
          WHERE anchor IS NOT NULL AND name = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map([name], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, anchor, content, ts) = row?;
        if anchors.contains(&anchor) {
            out.push(json!({"id": id, "content": content, "ts": ts}));
        }
    }
    empty(out)
}

fn history_global(conn: &Connection, name: Option<&str>) -> Result<Vec<Value>> {
    let name = required_name(name)?;
    let mut stmt = conn.prepare(
        "SELECT id, content, ts FROM memos
          WHERE anchor IS NULL AND name = ?1 ORDER BY id",
    )?;
    let rows = stmt.query_map([name], |r| {
        Ok(json!({
            "id": r.get::<_, i64>(0)?,
            "content": r.get::<_, String>(1)?,
            "ts": r.get::<_, String>(2)?,
        }))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    empty(out)
}

fn empty(rows: Vec<Value>) -> Result<Vec<Value>> {
    if rows.is_empty() {
        return Err(anyhow!("no memo with that name"));
    }
    Ok(rows)
}

fn required(session: Option<i64>) -> Result<i64> {
    session
        .filter(|id| *id > 0)
        .ok_or_else(|| anyhow!("a session is required for a current memo"))
}

fn required_name(name: Option<&str>) -> Result<&str> {
    name.ok_or_else(|| anyhow!("a memo name is required"))
}
