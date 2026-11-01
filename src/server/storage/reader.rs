//! The read side: one connection, one accessor per table. A named accessor decodes stored shapes
//! (`0`/`1` as booleans, JSON columns as JSON); a read plan returns the raw column values.

use std::path::Path;

use anyhow::{Context, Result, bail};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{Value, json};

use super::decode::{
    BLOCK_COLS, REQUEST_COLS, SESSION_COLS, TURN_COLS, TURN_FROM, block_from_row, request_from_row,
    session_meta, turn_from_row,
};
use super::shape::{Block, RequestMeta, SessionMeta, TurnRow};

/// One read connection. It lives as long as its user; the path never leaves this module.
pub struct Reader {
    conn: Connection,
}

impl Reader {
    /// Open a read connection. The database must exist: the writer creates it.
    pub fn open(path: &Path) -> Result<Reader> {
        let conn = Connection::open(path)
            .with_context(|| format!("cannot open database {}", path.display()))?;
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Ok(Reader { conn })
    }

    // ---- sessions ----

    /// Every session row.
    pub fn sessions(&self) -> Result<Vec<Value>> {
        self.rows(
            &format!("SELECT {SESSION_COLS} FROM sessions ORDER BY id"),
            rusqlite::params![],
            |r| Ok(session_json(session_meta(r)?)),
        )
    }

    /// One session row.
    pub fn session(&self, id: i64) -> Result<Option<Value>> {
        self.one(
            &format!("SELECT {SESSION_COLS} FROM sessions WHERE id = ?1"),
            [id],
            |r| Ok(session_json(session_meta(r)?)),
        )
    }

    // ---- the list ----

    /// The session's round list, in order: which turn sits where.
    pub fn list(&self, session: i64) -> Result<Vec<Value>> {
        self.rows(
            "SELECT idx, turn_id, is_compaction FROM session_turns
              WHERE session_id = ?1 ORDER BY idx",
            [session],
            |r| {
                Ok(json!({
                    "idx": r.get::<_, i64>(0)?,
                    "turn_id": r.get::<_, i64>(1)?,
                    "is_compaction": r.get::<_, i64>(2)? != 0,
                }))
            },
        )
    }

    // ---- turns ----

    /// Every turn the session's list holds, in list order.
    pub fn turns(&self, session: i64) -> Result<Vec<Value>> {
        self.rows(
            &format!(
                "SELECT {TURN_COLS} {TURN_FROM}
                  JOIN session_turns st ON st.turn_id = t.turn_id
                  WHERE st.session_id = ?1 ORDER BY st.idx"
            ),
            [session],
            |r| Ok(turn_json(turn_from_row(r)?)),
        )
    }

    /// One turn row.
    pub fn turn(&self, turn_id: i64) -> Result<Option<Value>> {
        self.one(
            &format!("SELECT {TURN_COLS} {TURN_FROM} WHERE t.turn_id = ?1"),
            [turn_id],
            |r| Ok(turn_json(turn_from_row(r)?)),
        )
    }

    /// Every block id the session's list references, as closed intervals in list order.
    pub fn ranges(&self, session: i64) -> Result<Vec<[i64; 2]>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {TURN_COLS} {TURN_FROM}
              JOIN session_turns st ON st.turn_id = t.turn_id
              WHERE st.session_id = ?1 ORDER BY st.idx"
        ))?;
        let found = stmt.query_map([session], turn_from_row)?;
        let mut out = Vec::new();
        for row in found {
            out.extend(row?.blocks);
        }
        Ok(out)
    }

    /// One round's block intervals, in order.
    pub fn turn_ranges(&self, turn_id: i64) -> Result<Vec<[i64; 2]>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {TURN_COLS} {TURN_FROM} WHERE t.turn_id = ?1"
        ))?;
        let mut found = stmt.query_map([turn_id], turn_from_row)?;
        match found.next() {
            Some(row) => Ok(row?.blocks),
            None => Ok(Vec::new()),
        }
    }

    /// One request archive row.
    pub fn request(&self, request_id: i64) -> Result<Option<Value>> {
        let found = self
            .conn
            .query_row(
                &format!("SELECT {REQUEST_COLS} FROM request_meta WHERE request_id = ?1"),
                [request_id],
                request_from_row,
            )
            .optional()?;
        found
            .map(|(id, _, meta)| request_json(id, meta))
            .transpose()
    }

    // ---- blocks ----

    /// The blocks these closed id ranges cover, in id order.
    pub fn blocks(&self, ranges: &[[i64; 2]]) -> Result<Vec<Value>> {
        let mut stmt = self.conn.prepare(&format!(
            "SELECT {BLOCK_COLS} FROM blocks
              WHERE block_id BETWEEN ?1 AND ?2 ORDER BY block_id"
        ))?;
        let mut out = Vec::new();
        for [lo, hi] in ranges {
            if lo > hi {
                bail!("reversed range: [{lo}, {hi}]");
            }
            let found = stmt.query_map([lo, hi], |r| Ok(block_json(block_from_row(r)?)))?;
            out.extend(found.collect::<rusqlite::Result<Vec<_>>>()?);
        }
        Ok(out)
    }

    /// One block.
    pub fn block(&self, id: i64) -> Result<Option<Value>> {
        self.one(
            &format!("SELECT {BLOCK_COLS} FROM blocks WHERE block_id = ?1"),
            [id],
            |r| Ok(block_json(block_from_row(r)?)),
        )
    }

    /// The artifact an oversized tool output spilled into.
    pub fn artifact(&self, block_id: i64) -> Result<Option<Value>> {
        self.one(
            "SELECT name, total_lines, content, hash, reference
               FROM artifacts WHERE block_id = ?1",
            [block_id],
            |r| {
                Ok(json!({
                    "block_id": block_id,
                    "name": r.get::<_, String>(0)?,
                    "total_lines": r.get::<_, i64>(1)?,
                    "content": r.get::<_, String>(2)?,
                    "hash": r.get::<_, String>(3)?,
                    "reference": r.get::<_, String>(4)?,
                }))
            },
        )
    }

    /// Every artifact row a reference spelling matches. Equal text may have
    /// several block rows; they carry the same reference and content.
    pub fn artifacts_by_ref(&self, reference: &str) -> Result<Vec<Value>> {
        self.rows(
            "SELECT block_id, name, total_lines, content, hash, reference
               FROM artifacts WHERE reference = ?1 ORDER BY block_id",
            [reference],
            |r| {
                Ok(json!({
                    "block_id": r.get::<_, i64>(0)?,
                    "name": r.get::<_, String>(1)?,
                    "total_lines": r.get::<_, i64>(2)?,
                    "content": r.get::<_, String>(3)?,
                    "hash": r.get::<_, String>(4)?,
                    "reference": r.get::<_, String>(5)?,
                }))
            },
        )
    }

    // ---- memos ----

    /// Every memo version, oldest first; `anchor` is null for the global scope.
    pub fn memos(&self) -> Result<Vec<Value>> {
        self.rows(
            "SELECT id, anchor, name, content, ts FROM memos ORDER BY id",
            rusqlite::params![],
            |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "anchor": r.get::<_, Option<i64>>(1)?,
                    "name": r.get::<_, String>(2)?,
                    "content": r.get::<_, String>(3)?,
                    "ts": r.get::<_, String>(4)?,
                }))
            },
        )
    }

    /// Every alias, oldest first.
    pub fn aliases(&self) -> Result<Vec<Value>> {
        self.rows(
            "SELECT id, anchor, name, command, ts FROM aliases ORDER BY id",
            rusqlite::params![],
            |r| {
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "anchor": r.get::<_, i64>(1)?,
                    "name": r.get::<_, String>(2)?,
                    "command": r.get::<_, String>(3)?,
                    "ts": r.get::<_, String>(4)?,
                }))
            },
        )
    }

    // ---- the working-directory trail ----

    /// Each turn's working directory, in turn order.
    pub fn cwd_trail(&self, session: i64) -> Result<Vec<Value>> {
        self.rows(
            "SELECT turn_id, cwd FROM cwd_history WHERE session_id = ?1 ORDER BY turn_id",
            [session],
            |r| {
                Ok(json!({
                    "turn_id": r.get::<_, i64>(0)?,
                    "cwd": r.get::<_, String>(1)?,
                }))
            },
        )
    }

    // ---- the generic plan ----

    /// Run one read plan. Columns are validated against the sheet; values are bound.
    pub fn query(&self, plan: &super::query::Query) -> Result<Vec<Value>> {
        use super::query::{Cond, Order};
        use rusqlite::types::Value as Sql;

        let columns: Vec<(&'static str, &'static str)> = if plan.columns.is_empty() {
            plan.sheet.columns().to_vec()
        } else {
            let mut out = Vec::with_capacity(plan.columns.len());
            for name in &plan.columns {
                match plan.sheet.column(name) {
                    Some(sql) => out.push((*name, sql)),
                    None => bail!("no column `{name}` in {}", plan.sheet.name()),
                }
            }
            out
        };
        let select = columns
            .iter()
            .map(|(logical, sql)| format!("{sql} AS \"{logical}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let mut sql = format!("SELECT {select} FROM {}", plan.sheet.name());
        let mut params: Vec<Sql> = Vec::new();
        let mut parts: Vec<String> = Vec::new();
        for cond in &plan.filter {
            match cond {
                Cond::Eq(column, value) => {
                    let column = column_of(plan, column)?;
                    parts.push(format!("{column} = ?"));
                    params.push(bind(value));
                }
                Cond::In(column, values) => {
                    let column = column_of(plan, column)?;
                    if values.is_empty() {
                        parts.push("0".to_string());
                        continue;
                    }
                    let marks = vec!["?"; values.len()].join(", ");
                    parts.push(format!("{column} IN ({marks})"));
                    params.extend(values.iter().map(bind));
                }
                Cond::Contains(column, text) => {
                    let column = column_of(plan, column)?;
                    parts.push(format!("{column} LIKE '%' || ? || '%'"));
                    params.push(Sql::Text(text.clone()));
                }
                Cond::Within(column, ranges) => {
                    let column = column_of(plan, column)?;
                    if ranges.is_empty() {
                        parts.push("0".to_string());
                        continue;
                    }
                    let each: Vec<String> = ranges
                        .iter()
                        .map(|_| format!("{column} BETWEEN ? AND ?"))
                        .collect();
                    parts.push(format!("({})", each.join(" OR ")));
                    for [lo, hi] in ranges {
                        params.push(Sql::Integer(*lo));
                        params.push(Sql::Integer(*hi));
                    }
                }
                Cond::Null(column) => {
                    parts.push(format!("{} IS NULL", column_of(plan, column)?));
                }
            }
        }
        if !parts.is_empty() {
            sql.push_str(" WHERE ");
            sql.push_str(&parts.join(" AND "));
        }
        if let Some((column, order)) = plan.order {
            let direction = match order {
                Order::Asc => "ASC",
                Order::Desc => "DESC",
            };
            sql.push_str(&format!(
                " ORDER BY {} {direction}",
                column_of(plan, column)?
            ));
        }
        if let Some(limit) = plan.limit {
            sql.push_str(" LIMIT ?");
            params.push(Sql::Integer(i64::try_from(limit).unwrap_or(i64::MAX)));
        }

        let mut stmt = self.conn.prepare(&sql)?;
        let found = stmt.query_map(rusqlite::params_from_iter(params.iter()), |row| {
            let mut object = serde_json::Map::new();
            for (index, (logical, _)) in columns.iter().enumerate() {
                object.insert((*logical).to_string(), cell(row.get_ref(index)?));
            }
            Ok(Value::Object(object))
        })?;
        Ok(found.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    // ---- the two shapes of a query ----

    fn rows(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
        row: impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<Value>,
    ) -> Result<Vec<Value>> {
        let mut stmt = self.conn.prepare(sql)?;
        let found = stmt.query_map(params, row)?;
        Ok(found.collect::<rusqlite::Result<Vec<_>>>()?)
    }

    fn one(
        &self,
        sql: &str,
        params: impl rusqlite::Params,
        row: impl FnOnce(&rusqlite::Row<'_>) -> rusqlite::Result<Value>,
    ) -> Result<Option<Value>> {
        Ok(self.conn.query_row(sql, params, row).optional()?)
    }
}

// ---- the read plan's helpers ----

fn column_of(plan: &super::query::Query, name: &str) -> Result<&'static str> {
    match plan.sheet.column(name) {
        Some(column) => Ok(column),
        None => bail!("no column `{name}` in {}", plan.sheet.name()),
    }
}

fn bind(value: &super::query::Val) -> rusqlite::types::Value {
    use super::query::Val;
    use rusqlite::types::Value as Sql;
    match value {
        Val::Int(number) => Sql::Integer(*number),
        Val::Text(text) => Sql::Text(text.clone()),
    }
}

fn cell(value: rusqlite::types::ValueRef<'_>) -> Value {
    use rusqlite::types::ValueRef;
    match value {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(number) => Value::from(number),
        ValueRef::Real(number) => Value::from(number),
        ValueRef::Text(text) => Value::String(String::from_utf8_lossy(text).into_owned()),
        ValueRef::Blob(_) => Value::Null,
    }
}

// ---- rows as JSON ----

fn session_json(row: SessionMeta) -> Value {
    json!({
        "id": row.id,
        "name": row.name,
        "started_at": row.started_at,
        "cwd": row.cwd,
        "provider": row.provider,
        "model": row.model,
        "anchor": row.anchor,
        "meta": row.meta,
    })
}

fn turn_json(row: TurnRow) -> Value {
    let usage = row.usage;
    json!({
        "turn_id": row.turn_id,
        "ranges": row.blocks,
        "stop_reason": row.stop_reason,
        "provider": row.provider,
        "model": row.model,
        "prompt_tokens": usage.map(|u| u.prompt_tokens),
        "completion_tokens": usage.map(|u| u.completion_tokens),
        "cached_tokens": usage.map(|u| u.cached_tokens),
        "cache_write_tokens": usage.map(|u| u.cache_write_tokens),
    })
}

fn request_json(request_id: i64, row: RequestMeta) -> Result<Value> {
    Ok(json!({
        "request_id": request_id,
        "model": row.model,
        "provider": row.provider,
        "system": row.system,
        "tools": json_column(&row.tools_json, "request tools")?,
        "baseline_prompt": row.baseline_prompt,
    }))
}

fn block_json(row: Block) -> Value {
    json!({
        "id": row.id,
        "kind": row.kind.as_str(),
        "content": row.content,
        "prefix": row.prefix,
        "suffix": row.suffix,
        "usage_total": row.usage_total,
        "usage_prompt": row.usage_prompt,
        "usage_completion": row.usage_completion,
        "usage_cached": row.usage_cached,
        "usage_cache_write": row.usage_cache_write,
        "usage_reasoning": row.usage_reasoning,
        "call_id": row.call_id,
        "name": row.name,
        "args": row.args,
        "intent": row.intent,
        "text": row.text,
        "first": row.first,
        "ok": row.ok,
        "result": row.result,
        "memo": row.memo,
        "duration_ms": row.duration_ms,
        "ts": row.ts,
    })
}

/// A column that holds JSON: an unreadable value is a broken row, never silently empty.
fn json_column(raw: &str, what: &str) -> Result<Value> {
    serde_json::from_str(raw).with_context(|| format!("a {what} column is unreadable"))
}
