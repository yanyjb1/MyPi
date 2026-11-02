//! Terminal size, handed to a zone when it starts and again whenever the terminal changes.

use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermSize {
    pub cols: u16,
    pub rows: u16,
}

impl TermSize {
    /// Read the size out of a `local / resized` frame.
    pub fn of(content: &Value) -> Option<TermSize> {
        Some(TermSize {
            cols: content.get("cols")?.as_u64()?.try_into().ok()?,
            rows: content.get("rows")?.as_u64()?.try_into().ok()?,
        })
    }
}
