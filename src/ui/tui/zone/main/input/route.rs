//! Where one line of input goes: a message, or a command with its owner.
//!
//! The command table is the shared unit at the crate root; this file only reads it.

use crate::commands::{self, CommandMeta};

/// One input line, classified.
pub enum Route<'a> {
    /// Not a command: it leaves as a message.
    Message(&'a str),
    /// A command the table knows, with its argument text.
    Command(&'static CommandMeta, &'a str),
}

pub fn classify(input: &str) -> Route<'_> {
    match commands::split(input) {
        Some((meta, args)) => Route::Command(meta, args),
        None => Route::Message(input),
    }
}
