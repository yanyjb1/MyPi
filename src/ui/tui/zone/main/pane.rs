//! Rows the main zone divides among its panes: the ask, the grant, the broadcast.

use crate::ui::tui::zone::TermSize;

/// A sub-zone the main zone hands rows to.
pub(crate) trait Pane {
    /// Rows this pane wants, given the terminal size.
    fn request_height(&mut self, _term: TermSize) -> u16 {
        0
    }

    /// The rows the main zone granted, plus the terminal size.
    fn assign(&mut self, term: TermSize, rows: u16);

    /// The main zone's row broadcast.
    fn on_height_changed(&mut self, _rows: u16) {}
}
