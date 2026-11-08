//! What the input zone asks the desk for, and what the desk hands back.

use crate::ui::tui::zone::MemoAsk;

/// One temp hand. The desk seats at most one at a time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Hire {
    /// The completion popup.
    Completion,
    /// The `/usage` panel.
    Usage,
    /// The `/memo` panel, opened at one ask.
    Memo(MemoAsk),
}

/// What a hand wants the editor to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    /// Replace the cursor's word with this text.
    Replace {
        from: usize,
        to: usize,
        text: String,
    },
}
