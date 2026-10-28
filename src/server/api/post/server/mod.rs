//! Server writes: the default model, and a full config reload.

use anyhow::Result;

use crate::server::api::driver::Driver;
use crate::server::api::post::{ModelId, PostDone};

pub(crate) mod default_model;
pub(crate) mod reload;

pub use crate::ipc::contract::ServerWrite;

pub(crate) fn apply(driver: &mut Driver, write: ServerWrite) -> Result<PostDone> {
    match write {
        ServerWrite::DefaultModel(id) => {
            default_model::apply(driver, id)?;
            Ok(PostDone::Done)
        }
        ServerWrite::Reload => reload::apply(driver),
    }
}
