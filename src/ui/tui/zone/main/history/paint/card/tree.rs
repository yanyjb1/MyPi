//! Tree rows: a branch per entry, its children under the spine.

use ratatui::style::Modifier;

use super::super::super::layout::line::{Ink, Lead, Row, Run};
use crate::ui::tui::theme::fixed;

const BRANCH: &str = "|-- ";
const LAST: &str = "'-- ";
const SPINE: &str = "|   ";
const BLANK: &str = "    ";

/// One entry of a tree.
pub(crate) fn entry(last: bool, runs: Vec<Run>) -> Row {
    let mut out = vec![Run::fixed(if last { LAST } else { BRANCH }, fixed::DIM)];
    out.extend(runs);
    Row::of(Lead::None, out)
}

/// One child line under an entry; `last` is the entry's own position.
pub(crate) fn child(last: bool, runs: Vec<Run>) -> Row {
    let mut out = vec![Run::fixed(if last { BLANK } else { SPINE }, fixed::DIM)];
    out.extend(runs);
    Row::of(Lead::None, out)
}

/// A muted value, for the text after a branch.
pub(crate) fn value(text: impl Into<String>) -> Run {
    Run::with(text, Some(Ink::Fixed(fixed::MUTED)), Modifier::empty())
}
