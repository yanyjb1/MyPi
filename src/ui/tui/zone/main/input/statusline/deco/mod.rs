//! The statusline's decos, and the table that declares them.
//! Adding one: a new file plus a line in [`ROW`]; removing one: delete that line.

pub mod activity;
pub mod cost;
pub mod cwd;
pub mod git;
pub mod model;
pub mod session;
pub mod usage;

use super::{ColorPolicy, Entry, PRIORITY_BASE, Side, Token};

/// The row, in declaration order; `StatusLine::new` sorts it by `(side, order)`.
pub const ROW: &[Entry] = &[
    Entry {
        id: "activity",
        side: Side::Left,
        order: 0,
        priority: PRIORITY_BASE,
        colors: ColorPolicy::Tokens(&[Token::Muted, Token::Accent, Token::Capsule]),
        make: activity::make,
    },
    Entry {
        id: "model",
        side: Side::Left,
        order: 10,
        priority: 80,
        colors: ColorPolicy::Tokens(&[Token::Accent, Token::Capsule]),
        make: model::make,
    },
    Entry {
        id: "cwd",
        side: Side::Left,
        order: 20,
        priority: 40,
        colors: ColorPolicy::Tokens(&[Token::Text, Token::Capsule]),
        make: cwd::make,
    },
    Entry {
        id: "git",
        side: Side::Left,
        order: 30,
        priority: 55,
        colors: ColorPolicy::Tokens(&[
            Token::GitClean,
            Token::GitDirty,
            Token::Gold,
            Token::Capsule,
        ]),
        make: git::make,
    },
    Entry {
        id: "cost",
        side: Side::Left,
        order: 40,
        priority: PRIORITY_BASE,
        colors: ColorPolicy::Tokens(&[Token::Gold, Token::Capsule]),
        make: cost::make,
    },
    Entry {
        id: "usage",
        side: Side::Flex,
        order: 0,
        priority: PRIORITY_BASE,
        colors: ColorPolicy::Tokens(&[Token::Accent]),
        make: usage::make,
    },
    Entry {
        id: "session",
        side: Side::Right,
        order: 0,
        priority: 60,
        colors: ColorPolicy::Tokens(&[Token::Accent, Token::Capsule]),
        make: session::make,
    },
];
