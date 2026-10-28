//! Delete: a child is refused; a parent takes its children with it.

use anyhow::{Result, bail};
use rusqlite::OptionalExtension;
use std::sync::atomic::Ordering;

use crate::server::api::call::session::block_ids;
use crate::server::api::driver::Driver;
use crate::server::storage::Reader;

pub(crate) fn apply(driver: &Driver, session: i64) -> Result<()> {
    let conn = Reader::open(&driver.db)?;
    let child: Option<i64> = conn
        .query_row(
            "SELECT subsession_id FROM subsessions WHERE subsession_id = ?1",
            [session],
            |r| r.get(0),
        )
        .optional()?;
    if child.is_some() {
        Reader::close(conn);
        bail!("a child session cannot be deleted on its own");
    }

    let anchors = block_ids(&conn, session)?;
    let mut stmt = conn.prepare("SELECT subsession_id, anchor FROM subsessions")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, i64>(1)?)))?;
    let mut children = Vec::new();
    for row in rows {
        let (child, anchor) = row?;
        if anchors.contains(&anchor) {
            children.push(child);
        }
    }
    drop(stmt);
    Reader::close(conn);

    for child in children {
        if let Some(flag) = driver.running.flag(child) {
            flag.store(true, Ordering::Relaxed);
        }
        driver.writer.delete_session(child)?;
        close_web(child);
        driver.sessions.forget(child);
    }

    driver.writer.delete_session(session)?;
    close_web(session);
    driver.sessions.forget(session);
    driver.relay.deleted(session);
    Ok(())
}

fn close_web(session: i64) {
    #[cfg(feature = "web")]
    crate::server::agent::tools::close_web_session(session);
    #[cfg(not(feature = "web"))]
    let _ = session;
}
