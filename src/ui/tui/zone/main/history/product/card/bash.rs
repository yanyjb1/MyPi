//! bash: the command, then its output.

use super::super::super::prefetch::color::code;
use super::super::super::row::{Lead, Row, Run};
use super::{ToolBlock, ToolRenderer, partial};
use crate::ui::tui::theme::fixed;

pub(crate) struct Bash;

impl ToolRenderer for Bash {
    fn call_body(&self, b: &ToolBlock<'_>) -> Vec<Row> {
        bash_call(b.args)
    }
}

fn bash_call(args: &str) -> Vec<Row> {
    let cmd = partial::text(args, "command").unwrap_or_default();
    if cmd.trim().is_empty() {
        return Vec::new();
    }
    let mut out = Vec::new();
    for (i, line) in cmd.lines().enumerate() {
        let mut runs = Vec::new();
        if i == 0 {
            runs.push(Run::fixed("$ ", fixed::DIM));
        }
        let hl = code::line(line, Some("sh"));
        if hl.runs.is_empty() {
            runs.push(Run::plain(line));
        } else {
            runs.extend(hl.runs);
        }
        out.push(Row::of(Lead::None, runs));
    }
    out
}
