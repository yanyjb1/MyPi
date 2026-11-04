//! The checklist's own product: it keeps its rows until the list or the width changes.

use super::super::layout::line::Row;
use super::super::paint::todo::{self, Task};
use crate::ui::tui::TodoFold;

#[derive(Default)]
pub(crate) struct TodoPanel {
    rows: Vec<Row>,
    width: usize,
    dirty: bool,
    laid: bool,
}

impl TodoPanel {
    /// The list changed.
    pub(crate) fn changed(&mut self) {
        self.dirty = true;
    }

    /// The width changed.
    pub(crate) fn dropped(&mut self) {
        self.dirty = true;
    }

    pub(crate) fn rows(
        &mut self,
        tasks: &[Task],
        fold: TodoFold,
        width: usize,
        budget: usize,
    ) -> &[Row] {
        if !self.laid || self.dirty || self.width != width {
            self.rows = todo::rows(tasks, fold, width, budget);
            self.width = width;
            self.dirty = false;
            self.laid = true;
        }
        &self.rows
    }
}
