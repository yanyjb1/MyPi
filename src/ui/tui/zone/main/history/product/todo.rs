//! The checklist's own product: it keeps its rows until the list or the width changes.

use super::super::row::Row;
pub(crate) mod rows;

pub(crate) use rows::Task;

use self::rows as todo;
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

    pub(crate) fn rows(&mut self, tasks: &[Task], fold: TodoFold, width: usize) -> &[Row] {
        if !self.laid || self.dirty || self.width != width {
            self.rows = todo::rows(tasks, fold, width);
            self.width = width;
            self.dirty = false;
            self.laid = true;
        }
        &self.rows
    }
}
