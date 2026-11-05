//! The statusline's decos, and the table that declares them.
//! Adding one: a new file plus a line in [`ROW`]; removing one: delete that line.

mod activity;
mod cost;
mod cwd;
pub(super) mod git;
mod hint;
mod model;
mod session;
mod usage;

use super::{Entry, PRIORITY_BASE, Side};

/// The row, in declaration order; `StatusLine::new` sorts it by `(side, order)`.
pub const ROW: &[Entry] = &[
    Entry {
        side: Side::Left,
        order: 0,
        priority: PRIORITY_BASE,
        make: activity::make,
    },
    Entry {
        side: Side::Left,
        order: 10,
        priority: 80,
        make: model::make,
    },
    Entry {
        side: Side::Left,
        order: 20,
        priority: 40,
        make: cwd::make,
    },
    Entry {
        side: Side::Left,
        order: 30,
        priority: 55,
        make: git::make,
    },
    Entry {
        side: Side::Left,
        order: 40,
        priority: PRIORITY_BASE,
        make: cost::make,
    },
    Entry {
        side: Side::Flex,
        order: 0,
        priority: 90,
        make: hint::make,
    },
    Entry {
        side: Side::Flex,
        order: 10,
        priority: PRIORITY_BASE,
        make: usage::make,
    },
    Entry {
        side: Side::Right,
        order: 0,
        priority: 60,
        make: session::make,
    },
];
