//! The display layer: where the sight line sits, and what it needs to draw.

use super::super::local::section::Local;
use super::super::prefetch::{At, Run};
use super::super::product::{Opts, Products};

/// Everything the window walks.
pub struct Scene<'a> {
    pub local: &'a Local,
    pub products: &'a mut Products,
    pub run: &'a mut Run,
    pub opts: &'a Opts<'a>,
    pub width: usize,
}

/// Which way the reader moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dir {
    Older,
    Newer,
}

/// The window: the sight line's place, and the rows the host gave it.
pub struct View {
    at: Option<At>,
    rows: u16,
    focus_line: u16,
}

impl Default for View {
    fn default() -> Self {
        View {
            at: None,
            rows: 0,
            focus_line: 3,
        }
    }
}

impl View {
    /// Follow the newest content.
    pub fn follow(&mut self) {
        self.at = None;
    }

    pub fn at(&self) -> Option<At> {
        self.at
    }

    pub fn set_at(&mut self, at: Option<At>) {
        self.at = at;
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    pub fn assign(&mut self, rows: u16) {
        self.rows = rows;
    }

    pub fn set_focus_line(&mut self, line: u16) {
        self.focus_line = line.max(1);
    }

    pub fn focus_line(&self) -> u16 {
        self.focus_line
    }
}
