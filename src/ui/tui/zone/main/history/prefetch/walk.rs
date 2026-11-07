//! The slot next to this one in render order.

use super::super::local::section::{Idx, Local};

/// The slot after this one, if any.
pub fn newer(local: &Local, idx: Idx) -> Option<Idx> {
    match idx {
        Idx::Fixed(num) => match local.at(num + 1) {
            Some(_) => Some(Idx::Fixed(num + 1)),
            None => live_first(local),
        },
        Idx::Live(at) => {
            let next = at + 1;
            (next < local.live().len() as u32).then_some(Idx::Live(next))
        }
    }
}

/// The slot before this one, if any.
pub fn older(local: &Local, idx: Idx) -> Option<Idx> {
    match idx {
        Idx::Fixed(num) => (num > local.oldest()?).then_some(Idx::Fixed(num - 1)),
        Idx::Live(at) => {
            if at == 0 {
                local.newest().map(Idx::Fixed)
            } else {
                Some(Idx::Live(at - 1))
            }
        }
    }
}

/// The first slot of the running turn.
fn live_first(local: &Local) -> Option<Idx> {
    (!local.live().is_empty()).then_some(Idx::Live(0))
}
