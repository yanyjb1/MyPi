//! Tool writes: memo add / mod / del.

use crate::ipc::contract::post::MemoWrite;
use anyhow::Result;

use crate::server::api::driver::Driver;

pub(crate) mod memo;

pub(crate) fn apply(driver: &Driver, write: MemoWrite) -> anyhow::Result<()> {
    memo::apply(driver, write)
}
