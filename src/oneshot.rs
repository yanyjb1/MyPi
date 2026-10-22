//! `mypi sessions` — list sessions without touching a socket: a one-shot query
//! opens the database for reading itself (`memo.md`: read-only work needs no lock).

use crate::server::api::call::sessions::display_name;
use crate::server::api::{self, Call, Reply};
use crate::server::config::Config;

/// `mypi sessions`: TSV rows — id, name, started, cwd.
pub fn run_sessions(cfg: &Config) -> anyhow::Result<()> {
    let Reply::SessionRows(rows) = api::call::answer(cfg, Call::Sessions { under: None })? else {
        anyhow::bail!("the sessions door answered something other than rows");
    };
    println!("id\tname\tstarted\tcwd");
    for row in rows {
        let meta = &row.meta;
        let name = meta
            .name
            .clone()
            .unwrap_or_else(|| display_name(meta, row.first_user.as_deref()));
        println!(
            "{}\t{}\t{}\t{}",
            meta.id,
            name,
            meta.started_at,
            meta.cwd.as_deref().unwrap_or(""),
        );
    }
    Ok(())
}
