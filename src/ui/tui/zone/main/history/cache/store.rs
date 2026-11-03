//! The two layers, and the keys they are built under.

use super::super::line::Row;
use super::super::wrap;
use super::rows::{Entry, Rows};
use super::text::Text;

/// Which layer a block goes through.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Class {
    /// Prose: parsed once, wrapped at the current width.
    Markdown,
    /// Cards: laid out whole at the current width.
    Whole,
}

/// Everything a width row depends on.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Key {
    width: usize,
    switches: (bool, bool),
    fold: usize,
}

pub(crate) struct Cache {
    text: Text,
    rows: Rows,
    key: Option<Key>,
}

impl Default for Cache {
    fn default() -> Self {
        Cache {
            text: Text::default(),
            rows: Rows::default(),
            key: None,
        }
    }
}

impl Cache {
    pub(crate) fn set_budget(&mut self, budget: usize) {
        self.text.set_budget(budget);
        self.rows.set_budget(budget);
    }

    /// Drop the width rows when their key stops holding. The parsed layer stays.
    pub(crate) fn sync(&mut self, width: usize, switches: (bool, bool), fold: usize) {
        let key = Key {
            width,
            switches,
            fold,
        };
        if self.key != Some(key) {
            self.rows.clear();
            self.key = Some(key);
        }
    }

    pub(crate) fn width(&self) -> usize {
        self.key.map(|key| key.width).unwrap_or(1)
    }

    /// One block's display rows, laid out now if they are not here.
    pub(crate) fn slot(
        &mut self,
        id: i64,
        class: Class,
        units: impl FnOnce() -> Vec<Vec<Row>>,
        whole: impl FnOnce() -> Vec<Row>,
    ) -> &Entry {
        if self.rows.known(id).is_none() {
            let width = self.width();
            let entry = match class {
                Class::Markdown => {
                    let mut rows: Vec<Row> = Vec::new();
                    let mut marks: Vec<usize> = Vec::new();
                    for segment in self.text.get(id, units) {
                        for natural in segment {
                            marks.push(rows.len());
                            rows.extend(wrap::wrap(natural, width));
                        }
                    }
                    Entry {
                        rows,
                        marks,
                        avail: width.saturating_sub(2).max(1),
                    }
                }
                Class::Whole => Entry {
                    rows: whole(),
                    marks: Vec::new(),
                    avail: 0,
                },
            };
            self.rows.insert(id, entry);
        }
        self.rows.entry(id)
    }

    pub(crate) fn height(&self, id: i64) -> Option<usize> {
        self.rows.known(id).map(Entry::height)
    }

    pub(crate) fn spot_of(&self, id: i64, row: usize) -> (usize, usize) {
        match self.rows.known(id) {
            Some(entry) => entry.spot_of(row),
            None => (0, 0),
        }
    }

    pub(crate) fn row_of(&self, id: i64, line: usize, off: usize) -> Option<usize> {
        self.rows.known(id).map(|entry| entry.row_of(line, off))
    }

    pub(crate) fn drop_block(&mut self, id: i64) {
        self.text.drop_block(id);
        self.rows.drop_block(id);
    }

    pub(crate) fn clear(&mut self) {
        self.text.clear();
        self.rows.clear();
        self.key = None;
    }
}
