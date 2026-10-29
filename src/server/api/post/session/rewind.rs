//! Rewind: prune the list back to a block.

use anyhow::Result;

use crate::ipc::contract::post::Cut;
use crate::server::api::driver::Driver;
use crate::server::storage::{Target, Writer};

pub(crate) fn prune(w: &Writer, target: Target) -> Result<crate::server::storage::Cut> {
    w.rewind(target)
}

pub(crate) fn apply(driver: &Driver, session: i64, turn: i64, block: i64) -> Result<Cut> {
    let cut = driver.rewind(Target {
        session,
        turn,
        block,
    })?;
    driver.relay.rewound(session, turn, block);
    Ok(cut_to_wire(cut))
}

fn cut_to_wire(cut: crate::server::storage::Cut) -> Cut {
    match cut {
        crate::server::storage::Cut::Boundary { turn } => Cut::Boundary { turn },
        crate::server::storage::Cut::Rescued { turn } => Cut::Rescued { turn },
        crate::server::storage::Cut::Retyped { turn } => Cut::Retyped { turn },
    }
}
