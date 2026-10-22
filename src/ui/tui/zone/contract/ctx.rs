//! What every delivery carries: the read model, the size, and the memo window.

use crate::ui::session::View;
use crate::ui::tui::app::Memo;

/// Terminal size, in columns and rows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TermSize {
    pub cols: u16,
    pub rows: u16,
}

/// Handed down with every delivery.
pub struct Ctx<'a> {
    pub view: &'a View<'a>,
    pub size: TermSize,
    /// `Some` only during handover and takeover.
    pub memo: Option<&'a Memo>,
}
