//! Rewind: prune the list back to a block.

use anyhow::Result;

use crate::server::api::driver::Driver;
use crate::server::storage::{Cut, Target, Writer};

pub(crate) fn prune(w: &Writer, target: Target) -> Result<Cut> {
    w.rewind(target)
}

pub(crate) fn apply(driver: &Driver, session: i64, turn: i64, block: i64) -> Result<()> {
    driver.rewind(Target {
        session,
        turn,
        block,
    })?;
    driver.relay.rewound(session, turn, block);
    Ok(())
}
