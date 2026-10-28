//! `mypi sessions` — list sessions through the running server, never the database.

use std::path::Path;

use crate::ipc::client::Client;
use crate::ipc::contract::{Call, Reply, ServerAsk, ServerReply, Viewer, display_name};
use crate::ipc::endpoint;

/// `mypi sessions`: TSV rows — id, name, started, cwd.
pub fn run_sessions(file: &Path) -> anyhow::Result<()> {
    let endpoint = endpoint::read(file)?;
    let client = Client::connect(endpoint, Viewer("oneshot".to_string()))?;
    let Reply::Server(ServerReply::Sessions(rows)) =
        client.call(Call::Server(ServerAsk::Sessions { under: None }))?
    else {
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
