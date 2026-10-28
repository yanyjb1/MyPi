//! Memo reads: names, current content, and the whole history.

use anyhow::{Result, anyhow};
use rusqlite::Connection;
use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::server::agent::tools::{MemoRead, MemoReadOp, MemoVersion, MemoView};
use crate::server::api::call::ctx::Ctx;
use crate::server::api::call::session::block_ids;
use crate::server::storage::Reader;

use super::{MemoOp, MemoScope};

pub use crate::ipc::contract::{MemoData, MemoEntry};

pub(crate) fn answer(
    ctx: &Ctx,
    session: Option<i64>,
    op: MemoOp,
    scope: MemoScope,
    name: Option<String>,
) -> Result<MemoData> {
    match op {
        MemoOp::List => names(ctx.reader, session, scope).map(MemoData::Names),
        MemoOp::Show => content(ctx.reader, session, scope, name).map(MemoData::Content),
        MemoOp::History => history(ctx.reader, session, scope, name).map(MemoData::History),
    }
}

/// The read port handed to the `memo` tool; it owns no connection between calls.
pub(crate) struct MemoStore {
    db: PathBuf,
}

impl MemoStore {
    pub(crate) fn new(db: PathBuf) -> Self {
        Self { db }
    }
}

impl MemoRead for MemoStore {
    fn read(
        &self,
        session: i64,
        scope: crate::server::storage::MemoScope,
        op: MemoReadOp,
        name: Option<&str>,
    ) -> Result<MemoView> {
        let conn = Reader::open(&self.db)?;
        let scope = match scope {
            crate::server::storage::MemoScope::Current => MemoScope::Current,
            crate::server::storage::MemoScope::Global => MemoScope::Global,
        };
        let out = match op {
            MemoReadOp::List => MemoView::Names(names(&conn, Some(session), scope)?),
            MemoReadOp::Show => MemoView::Content(content(
                &conn,
                Some(session),
                scope,
                name.map(|value| value.to_string()),
            )?),
            MemoReadOp::History => MemoView::History(
                history(
                    &conn,
                    Some(session),
                    scope,
                    name.map(|value| value.to_string()),
                )?
                .into_iter()
                .map(|entry| MemoVersion {
                    id: entry.id,
                    content: entry.content,
                    ts: entry.ts,
                })
                .collect(),
            ),
        };
        Reader::close(conn);
        Ok(out)
    }
}

fn names(conn: &Connection, session: Option<i64>, scope: MemoScope) -> Result<Vec<String>> {
    let mut out = BTreeSet::new();
    match scope {
        MemoScope::Current => {
            let session = current_session(session)?;
            let anchors = block_ids(conn, session)?;
            let mut stmt =
                conn.prepare("SELECT anchor, name FROM memos WHERE anchor IS NOT NULL")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (anchor, name) = row?;
                if anchors.contains(&anchor) {
                    out.insert(name);
                }
            }
        }
        MemoScope::Global => {
            let mut stmt = conn.prepare("SELECT DISTINCT name FROM memos WHERE anchor IS NULL")?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
            for row in rows {
                out.insert(row?);
            }
        }
    }
    Ok(out.into_iter().collect())
}

fn content(
    conn: &Connection,
    session: Option<i64>,
    scope: MemoScope,
    name: Option<String>,
) -> Result<String> {
    let name = name.ok_or_else(|| anyhow!("a memo name is required"))?;
    match scope {
        MemoScope::Current => {
            let session = current_session(session)?;
            let anchors = block_ids(conn, session)?;
            let mut stmt = conn.prepare(
                "SELECT anchor, content FROM memos
                  WHERE anchor IS NOT NULL AND name = ?1 ORDER BY id DESC",
            )?;
            let rows = stmt.query_map([name.clone()], |r| {
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
        MemoScope::Global => conn
            .query_row(
                "SELECT content FROM memos
                  WHERE anchor IS NULL AND name = ?1 ORDER BY id DESC LIMIT 1",
                [name],
                |r| r.get(0),
            )
            .map_err(|_| anyhow!("no global memo with that name")),
    }
}

fn history(
    conn: &Connection,
    session: Option<i64>,
    scope: MemoScope,
    name: Option<String>,
) -> Result<Vec<MemoEntry>> {
    let name = name.ok_or_else(|| anyhow!("a memo name is required"))?;
    let mut out = Vec::new();
    match scope {
        MemoScope::Current => {
            let session = current_session(session)?;
            let anchors = block_ids(conn, session)?;
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
            for row in rows {
                let (id, anchor, content, ts) = row?;
                if anchors.contains(&anchor) {
                    out.push(MemoEntry { id, content, ts });
                }
            }
        }
        MemoScope::Global => {
            let mut stmt = conn.prepare(
                "SELECT id, content, ts FROM memos
                  WHERE anchor IS NULL AND name = ?1 ORDER BY id",
            )?;
            let rows = stmt.query_map([name], |r| {
                Ok(MemoEntry {
                    id: r.get(0)?,
                    content: r.get(1)?,
                    ts: r.get(2)?,
                })
            })?;
            for row in rows {
                out.push(row?);
            }
        }
    }
    if out.is_empty() {
        return Err(anyhow!("no memo with that name"));
    }
    Ok(out)
}

fn current_session(session: Option<i64>) -> Result<i64> {
    session
        .filter(|id| *id > 0)
        .ok_or_else(|| anyhow!("a session is required for a current memo"))
}
