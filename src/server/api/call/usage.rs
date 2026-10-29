//! `usage` — tokens per model, with the cache-hit share.

use anyhow::Result;
use serde_json::{Value, json};

use crate::server::api::{Api, Refuse};
use crate::server::storage::Reader;

pub(crate) fn answer(api: &Api, session: i64) -> Result<Value, Refuse> {
    let conn = Reader::open(api.db())?;
    let out = build(&conn, session);
    Reader::close(conn);
    out
}

fn build(conn: &rusqlite::Connection, session: i64) -> Result<Value, Refuse> {
    let mut stmt = conn.prepare(
        "SELECT r.provider, r.model, COUNT(*),
                COALESCE(SUM(t.prompt_tokens), 0),
                COALESCE(SUM(t.completion_tokens), 0),
                COALESCE(SUM(t.cached_tokens), 0),
                COALESCE(SUM(t.cache_write_tokens), 0)
           FROM turns t JOIN session_turns s ON s.turn_id = t.turn_id
           JOIN request_meta r ON r.request_id = t.request_id
          WHERE s.session_id = ?1
          GROUP BY r.provider, r.model
          ORDER BY MIN(s.idx)",
    )?;
    let rows = stmt.query_map([session], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, i64>(2)?.max(0),
            r.get::<_, i64>(3)?.max(0),
            r.get::<_, i64>(4)?.max(0),
            r.get::<_, i64>(5)?.max(0),
            r.get::<_, i64>(6)?.max(0),
        ))
    })?;

    let mut groups = Vec::new();
    let mut prompt = 0i64;
    let mut completion = 0i64;
    let mut cached = 0i64;
    let mut written = 0i64;
    for row in rows {
        let (provider, model, rounds, p, c, k, w) = row?;
        prompt += p;
        completion += c;
        cached += k;
        written += w;
        groups.push(json!({
            "provider": provider,
            "model": model,
            "rounds": rounds,
            "prompt_tokens": p,
            "completion_tokens": c,
            "cached_tokens": k,
            "cache_write_tokens": w,
            "price": Value::Null,
        }));
    }

    let total = prompt + completion;
    Ok(json!({
        "prompt_tokens": prompt,
        "completion_tokens": completion,
        "cached_tokens": cached,
        "cache_write_tokens": written,
        "total_tokens": total,
        "cache_hit_rate": share(cached, total),
        "groups": groups,
    }))
}

fn share(part: i64, whole: i64) -> f64 {
    if whole <= 0 {
        return 0.0;
    }
    part as f64 / whole as f64
}
