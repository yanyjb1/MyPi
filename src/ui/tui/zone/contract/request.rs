//! What a zone asks the loop to do. Screen changes never appear here.

use crate::ui::tui::zone::main::history::ask::Ask;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Request {
    Submit(String),
    Interrupt,
    /// Cut back to a block; user text rides along for the clipboard.
    Rewind {
        turn: i64,
        block: i64,
        text: Option<String>,
    },
    /// A history note: jump, fetch, or settle.
    View(Ask),
    /// The picker's list, scoped to a directory (`None` = every session).
    List {
        under: Option<std::path::PathBuf>,
    },
    Attach(i64),
    Delete(i64),
    /// Leave the current session without attaching another.
    Detach,
    /// Raw material for the tree, block ids `[start, end)`.
    TreeRange {
        start: i64,
        end: i64,
    },
}
