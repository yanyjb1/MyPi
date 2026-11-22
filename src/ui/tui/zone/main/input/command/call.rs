//! What one command reads, and what it answers.

use crate::ui::tui::zone::Frame;
use crate::ui::tui::zone::main::Draft;

use super::super::{ExitRequest, Notice};
use crate::ui::tui::zone::main::desk::{Effect, Hire};

/// One command's input.
pub struct Call<'a> {
    /// The text after the command name, trimmed.
    pub args: &'a str,

    /// The session this front end follows; `None` = a draft.
    pub session: Option<i64>,

    /// Where the conversation stands.
    pub cwd: &'a str,

    /// The rungs the model this front end shows declares; empty takes no effort.
    pub efforts: &'a [String],
    /// What `~` stands for in this process.
    pub home: &'a str,
    /// The draft every zone shares: a command without a session writes here.
    pub draft: &'a Draft,
}

/// One write: the letter as the command spelled it, and what to say once it lands.
pub struct Write {
    pub letter: Frame,
    // The input zone's own bookkeeping for this write, if it keeps any.
    pub effect: Option<Effect>,
    pub notice: Notice,
}

impl Write {
    /// A write that leaves nothing for this zone to remember.
    pub fn new(letter: Frame, notice: Notice) -> Write {
        Write {
            letter,
            effect: None,
            notice,
        }
    }
}

/// What one command answers.
pub enum Done {
    /// One write, and what to say once it lands.
    Write(Write),

    /// One message, carrying the context the command built.
    Submit {
        prefix: Option<String>,
        text: String,
    },

    /// Something only this front end does.
    Exit(ExitRequest),

    /// One temp hand.
    Hire(Hire),

    /// A refusal: one line for the history zone, and the box is cleared.
    Notice(Notice),

    /// An argument is missing or unusable: the line stays in the box.
    Hold,
}
