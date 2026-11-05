//! What one command reads, and what it answers.

use crate::ui::tui::zone::Request;
use crate::ui::tui::zone::main::desk::Hire;
use crate::ui::tui::zone::main::input::ExitRequest;

/// One command's input.
pub struct Call<'a> {
    /// The text after the command name, trimmed.
    pub args: &'a str,
    /// A conversation is open.
    pub attached: bool,
}

/// What one command answers.
pub enum Done {
    /// One session action.
    Intent(Request),
    /// Something only this front end does.
    Exit(ExitRequest),
    /// One temp hand.
    Hire(Hire),
    /// The profile a session will open with.
    Profile(Option<String>),
    /// A line for the input zone; the command did not run.
    Hint(&'static str),
    /// An argument is missing or unusable: the line stays in the box.
    Hold,
}
