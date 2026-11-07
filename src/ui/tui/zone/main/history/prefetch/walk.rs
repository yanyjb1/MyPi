//! The slot next to this one in render order.

use super::super::local::section::{Idx, Local};

/// The slot after this one, if any.
pub fn newer(local: &Local, idx: Idx) -> Option<Idx> {
    match idx {
        Idx::Fixed(num) => match local.at(num + 1) {
            Some(_) => Some(Idx::Fixed(num + 1)),
            None => turn_first(local),
        },
        Idx::Turn(at) => {
            let next = at + 1;
            (next < local.running().len() as u32).then_some(Idx::Turn(next))
        }
    }
}

/// The slot before this one, if any.
pub fn older(local: &Local, idx: Idx) -> Option<Idx> {
    match idx {
        Idx::Fixed(num) => (num > local.oldest()?).then_some(Idx::Fixed(num - 1)),
        Idx::Turn(at) => {
            if at == 0 {
                local.newest().map(Idx::Fixed)
            } else {
                Some(Idx::Turn(at - 1))
            }
        }
    }
}

/// The first slot of the running turn.
fn turn_first(local: &Local) -> Option<Idx> {
    (!local.running().is_empty()).then_some(Idx::Turn(0))
}
