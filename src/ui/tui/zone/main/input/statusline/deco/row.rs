// The statusline's row: every deco, where it sits, and how long it survives a narrow width.

use super::super::{Entry, PRIORITY_BASE, Side};

// The row, in declaration order; `StatusLine::new` sorts it by `(side, order)`.
pub const ROW: &[Entry] = &[
    Entry {
        side: Side::Left,
        order: 0,
        priority: PRIORITY_BASE,
        make: super::activity::make,
    },
    Entry {
        side: Side::Left,
        order: 10,
        priority: 80,
        make: super::model::make,
    },
    Entry {
        side: Side::Left,
        order: 20,
        priority: 40,
        make: super::cwd::make,
    },
    Entry {
        side: Side::Left,
        order: 30,
        priority: 55,
        make: super::git::make,
    },
    Entry {
        side: Side::Left,
        order: 40,
        priority: PRIORITY_BASE,
        make: super::cost::make,
    },
    Entry {
        side: Side::Flex,
        order: 10,
        // The bar yields before the name, the price and the directory do.
        priority: 30,
        make: super::usage::make,
    },
    Entry {
        side: Side::Right,
        order: 0,
        priority: PRIORITY_BASE,
        make: super::session::make,
    },
];
