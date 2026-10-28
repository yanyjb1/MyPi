//! Server queries: config values, completion pools, and the resume list.

use anyhow::Result;

use crate::ipc::contract::ServerReply;
use crate::server::api::call::ctx::Ctx;

pub mod config;
pub mod pools;
pub mod sessions;

pub use crate::ipc::contract::{CandidatePools, ConfigValue, ServerAsk, SessionRow};

pub(crate) fn answer(ctx: &Ctx, ask: ServerAsk) -> Result<ServerReply> {
    match ask {
        ServerAsk::Config { key } => Ok(ServerReply::Config(config::value(ctx, &key)?)),
        ServerAsk::Pools => Ok(ServerReply::Pools(pools::pools(ctx.cfg))),
        ServerAsk::Sessions { under } => Ok(ServerReply::Sessions(sessions::rows(
            ctx.reader,
            under.as_deref(),
        )?)),
    }
}
