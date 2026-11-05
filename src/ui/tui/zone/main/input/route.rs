//! Where one line of input goes: a message, or a command with its owner.
//!
//! The command table is the input zone's own; this file only reads it.

use super::command;
use crate::ui::command::CommandMeta;

/// One input line, classified.
pub enum Route<'a> {
    /// Not a command: it leaves as a message.
    Message,
    /// A command the table knows, with its argument text.
    Command(&'static CommandMeta, &'a str),
}

pub fn classify(input: &str) -> Route<'_> {
    match command::split(input) {
        Some((meta, args)) => Route::Command(meta, args),
        None => Route::Message,
    }
}
