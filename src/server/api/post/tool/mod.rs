//! Tool writes: memo add / mod / del.

use anyhow::Result;

use crate::server::api::call::tool::MemoScope;
use crate::server::api::driver::Driver;
use crate::server::api::post::PostDone;

pub(crate) mod memo;

pub use crate::ipc::contract::{MemoWrite, MemoWriteOp, ToolWrite};

pub(crate) fn apply(driver: &Driver, write: ToolWrite) -> Result<PostDone> {
    match write {
        ToolWrite::Memo(write) => memo::apply(driver, write)?,
    }
    Ok(PostDone::Done)
}
