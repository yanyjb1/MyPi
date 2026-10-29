//! The session's row, plus the live working directory no row carries.

use anyhow::Result;

use crate::ipc::contract::call;
use crate::server::api::call::ctx::Ctx;
use crate::server::api::call::server::sessions::SESSION_COLS;
use crate::server::api::convert;
use crate::server::storage::decode::session_meta;

pub(crate) fn answer(ctx: &Ctx, session: i64) -> Result<call::meta::Reply> {
    let meta = ctx.reader.query_row(
        &format!("SELECT {SESSION_COLS} FROM sessions WHERE id = ?1"),
        [session],
        session_meta,
    )?;
    let cwd = ctx.sessions.cwd(ctx.cfg, ctx.reader, session)?;
    Ok(call::meta::Reply {
        meta: convert::session_meta(meta),
        cwd,
    })
}
