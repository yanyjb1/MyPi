//! What the input zone asks the desk for, and what the desk hands back.

/// One temp hand. The desk seats at most one at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Hire {
    /// The completion popup.
    Completion,
    /// The `/usage` panel.
    Usage,
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
