//! A compaction round: a banner at each end, the summary folded behind Ctrl + O.

use crate::ui::lang::Lang;
use crate::ui::tui::text::{display_width, take_width};
use crate::ui::tui::theme::fixed;
use crate::ui::tui::zone::main::history::local::content::Block;
use crate::ui::tui::zone::main::history::markdown;
use crate::ui::tui::zone::main::history::row::{Ink, Lead, Row, Run, split};
use crate::ui::tui::zone::main::num;

use super::fold;
use super::opts::Opts;

/// The banner a running compaction opens with.
const OPEN: &str = "compaction started";
/// What the row below it says while the summary is still arriving.
const STREAMING: &str = "summarizing";
/// What the same row says once it is done.
const WRITTEN: &str = "summary";
/// The banner a finished round closes with.
const CLOSE: &str = "compacted";
/// The running label, out of the product's hands: the pin prints it.
pub(crate) const LABEL: &str = "COMPACTING";

/// What one round of the compaction knows about itself, kept by block.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Mark {
    /// This process opened the round, so its opening banner stays under the closing one.
    pub opened: bool,
    /// The context as it stood before, when this process measured it.
    pub before: Option<u64>,
    /// The context the round left behind; the gauge's estimate, taken on the same terms.
    pub after: Option<u64>,
}

/// The running round's rows: the opening banner, then the summary it is writing.
pub(crate) fn running(block: &Block, opts: &Opts<'_>, width: usize) -> Vec<Row> {
    let mut out = vec![banner(OPEN, width)];
    out.extend(summary(block, opts, width, STREAMING));
    out
}

/// A finished round's rows: the banner it opened with when this process watched it start,
/// the summary, then the banner it closed with.
pub(crate) fn done(block: &Block, mark: Mark, opts: &Opts<'_>, width: usize) -> Vec<Row> {
    let mut out = Vec::new();
    if mark.opened {
        out.push(banner(OPEN, width));
    }
    out.extend(summary(block, opts, width, WRITTEN));
    out.push(banner(&closing(mark, opts.lang), width));
    out
}

/// The summary, folded to one word until Ctrl + O asks for it.
fn summary(block: &Block, opts: &Opts<'_>, width: usize, folded: &str) -> Vec<Row> {
    if !opts.tool {
        return vec![Row::text(
            Lead::None,
            folded,
            Some(Ink::Fixed(fixed::SYSTEM)),
        )];
    }
    let text = block
        .content
        .as_deref()
        .or(block.text.as_deref())
        .unwrap_or_default();
    body(text, width)
}

/// The summary at this width: markdown spans, then the terminal's row breaks.
fn body(text: &str, width: usize) -> Vec<Row> {
    let mut out: Vec<Row> = Vec::new();
    for line in text.lines() {
        let mut laid = markdown::rows(line);
        let runs = laid.pop().map(|row| row.runs).unwrap_or_default();
        out.extend(split(&runs, &fold::breaks(line, width), Lead::None));
    }
    out
}

/// `compacted`, and the two figures when this process measured them.
fn closing(mark: Mark, lang: Lang) -> String {
    match (mark.before, mark.after) {
        (Some(before), Some(after)) => format!(
            "{CLOSE} · {} -> {}",
            num::compact(before, lang),
            estimate(after, lang)
        ),
        (None, Some(after)) => format!("{CLOSE} · {}", estimate(after, lang)),
        _ => CLOSE.to_string(),
    }
}

/// The aftermath is the gauge's estimate, and it reads as one.
fn estimate(tokens: u64, lang: Lang) -> String {
    format!("~{}", num::compact(tokens, lang))
}

/// `>>>---label---<<<`: the arrow runs narrow tier by tier, the rule takes what is left.
fn banner(label: &str, width: usize) -> Row {
    let label_width = display_width(label);
    for arrows in [3usize, 2, 1] {
        // Two arrow runs, the label, and the one space the closing rule is set off by.
        let Some(rest) = width.checked_sub(2 * arrows + label_width + 1) else {
            continue;
        };
        if rest < 2 {
            continue;
        }
        let left = rest / 2;
        let right = rest - left;
        return Row::of(
            Lead::None,
            vec![
                Run::fixed(">".repeat(arrows), fixed::DIM),
                Run::fixed("-".repeat(left), fixed::DIM),
                Run::fixed(label, fixed::MUTED),
                Run::fixed(format!(" {}", "-".repeat(right)), fixed::DIM),
                Run::fixed("<".repeat(arrows), fixed::DIM),
            ],
        );
    }
    // Too narrow to frame: the bare label, cut to what the terminal has.
    let (text, _) = take_width(label, width);
    Row::text(Lead::None, text, Some(Ink::Fixed(fixed::MUTED)))
}
