//! Call — one question, one answer, read-only.

use anyhow::Result;

use crate::ipc::contract::Viewer;
use crate::server::api::driver;
use crate::server::config::Config;
use crate::server::storage::Reader;

pub(crate) mod ctx;
pub mod reply;
pub mod server;
pub mod session;
pub mod tool;

pub use crate::ipc::contract::Call;
pub use reply::Reply;

use ctx::Ctx;

/// Answer one question on a read connection.
pub fn answer(cfg: &Config, viewer: &Viewer, call: Call) -> Result<Reply> {
    let state = driver::state(cfg)?;
    let state = state.lock().unwrap_or_else(|e| e.into_inner());
    let conn = Reader::open(&state.db)?;
    let ctx = Ctx {
        cfg: &state.cfg,
        reader: &conn,
        attached: state.relay.session_of(viewer),
        sessions: &state.sessions,
        running: &state.running,
    };
    let out = dispatch(&ctx, call);
    Reader::close(conn);
    out
}

fn dispatch(ctx: &Ctx, call: Call) -> Result<Reply> {
    match call {
        Call::Session { session, ask } => Ok(Reply::Session(session::answer(ctx, session, ask)?)),
        Call::Tool(ask) => Ok(Reply::Tool(tool::answer(ctx, ask)?)),
        Call::Server(ask) => Ok(Reply::Server(server::answer(ctx, ask)?)),
    }
}
