//! Export current memos before their session is deleted.

use std::path::PathBuf;

use anyhow::{Context, Result};
use rusqlite::Connection;

use super::super::write::shared::{in_ranges, session_blocks};

/// Write one markdown file per session; `None` = there is nothing to export.
pub(crate) fn current(conn: &Connection, session: i64) -> Result<Option<PathBuf>> {
    let ranges = session_blocks(conn, session)?;
    if ranges.is_empty() {
        return Ok(None);
    }
    let mut stmt = conn.prepare(
        "SELECT id, anchor, name, content FROM memos WHERE anchor IS NOT NULL ORDER BY id",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((
            r.get::<_, i64>(0)?,
            r.get::<_, i64>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
        ))
    })?;
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for row in rows {
        let (_, anchor, name, content) = row?;
        if !in_ranges(&ranges, anchor) {
            continue;
        }
        match groups.iter_mut().find(|(n, _)| n.as_str() == name.as_str()) {
            Some((_, contents)) => contents.push(content),
            None => groups.push((name, vec![content])),
        }
    }
    if groups.is_empty() {
        return Ok(None);
    }
    let home = crate::server::platform::env::home().context("no home directory for memo export")?;
    let dir = home.join("Downloads");
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("cannot create export directory {}", dir.display()))?;
    let path = dir.join(format!("memos-{session}.md"));
    let mut out = format!("# Memos for session {session}\n\n");
    for (name, contents) in groups {
        out.push_str(&format!("## {name}\n\n"));
        for content in contents {
            out.push_str(&content);
            out.push_str("\n\n");
        }
    }
    std::fs::write(&path, out)
        .with_context(|| format!("cannot write memo export {}", path.display()))?;
    Ok(Some(path))
}
