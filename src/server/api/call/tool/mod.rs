//! Tool queries: the session's list, memo reads, and the child sessions it spawned.

use anyhow::Result;

use crate::ipc::contract::ToolReply;
use crate::server::api::call::ctx::Ctx;
use crate::server::api::convert;

pub mod children;
pub mod memo;
pub mod todo;

pub use crate::ipc::contract::{
    ChildRow, ChildState, MemoData, MemoEntry, MemoOp, MemoScope, ToolAsk,
};

pub(crate) fn answer(ctx: &Ctx, ask: ToolAsk) -> Result<ToolReply> {
    match ask {
        ToolAsk::Todo { session } => Ok(ToolReply::Todo(
            todo::list(ctx.reader, session)?
                .into_iter()
                .map(convert::todo_phase)
                .collect(),
        )),
        ToolAsk::Memo {
            session,
            op,
            scope,
            name,
        } => Ok(ToolReply::Memo(memo::answer(
            ctx, session, op, scope, name,
        )?)),
        ToolAsk::Children { session } => Ok(ToolReply::Children(children::list(ctx, session)?)),
    }
}
