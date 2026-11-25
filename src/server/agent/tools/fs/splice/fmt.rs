// The language's formatter as one pass over the merged text, with a comment around every block so the answer can name the lines each one became. Nothing here withholds an edit.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash as _, Hasher as _};
use std::path::Path;
use std::sync::Arc;

use super::edit::{self, Edit};

use crate::server::agent::tools::fs::lang::Format;
use crate::server::agent::tools::fs::lang::verdict::Outcome;
use crate::server::platform::stop::Stop;

// The word a change's wrapper comment starts with. The comment carries a fingerprint of the snapshot, so a line of the file that reads like a marker cannot be mistaken for one.
const MARK: &str = "mypi:splice";

// What the formatter made of one merged change.
pub(crate) struct Settled {
    // The text to write: the formatter's own when it agreed, the raw splice when it did not.
    pub text: String,

    // What the formatter said about that text.
    pub word: Word,

    // Where each change's own lines ended up, in the order the changes were handed over.
    pub landings: Vec<edit::Place>,

    // The edits whose block the parser blames on its own, with its words: a report, never a filter. Positions into the edits handed over.
    pub blamed: Vec<(usize, String)>,
}

// The formatter's verdict, as the answer speaks it.
pub(crate) enum Word {
    // No formatter was involved.
    Plain,

    // The formatter laid the merged text out.
    Formatted,

    // The formatter ran and said the merged text does not parse.
    Unparsed(String),

    // The formatter did not finish or could not run: the text was written as it stands.
    Undecided(String),
}

// Every edit spliced into the snapshot once with a comment around its block, then formatted in one pass. The comments are the only way back from the formatter's output to each block's lines, so the text that is written is that output without them. A text the formatter cannot parse is written as it stands; each block is then tried on its own only to say which one it blames. `None` = the turn was stopped, so nothing is written.
pub(crate) fn settle(
    base: &str,
    edits: &[Edit],
    format: Format,
    path: &Path,
    stop: &Arc<Stop>,
) -> Option<Settled> {
    let tag = tag(base);
    let marked: Vec<Edit> = edits
        .iter()
        .enumerate()
        .map(|(at, change)| change.marked(&tag, at))
        .collect();
    let picked: Vec<&Edit> = marked.iter().collect();
    match format(&edit::spliced(base, &picked), path, stop) {
        Outcome::Formatted(text) => {
            let landings = marks(&text, &tag);
            let (text, dropped) = unmarked(&text, &tag, &landings);
            let landings = landings
                .into_iter()
                .map(|place| place.map(|(first, last)| placed(first, last, &dropped)))
                .collect();
            Some(Settled {
                text,
                word: Word::Formatted,
                landings,
                blamed: Vec::new(),
            })
        }
        Outcome::Stopped => None,
        Outcome::Undecided(why) => Some(Settled {
            text: raw(base, edits),
            word: Word::Undecided(why),
            landings: edit::shifted(edits),
            blamed: Vec::new(),
        }),
        Outcome::Unparsed(why) => {
            let blamed = match edits.len() {
                // One edit is the whole text: the parser has already named it.
                1 => vec![(0, why.message.clone())],
                _ => blame(base, edits, format, path, stop)?,
            };
            Some(Settled {
                text: raw(base, edits),
                word: Word::Unparsed(why.message),
                landings: edit::shifted(edits),
                blamed,
            })
        }
    }
}

// The snapshot with every change in place, unformatted: what is written when the formatter did not lay it out.
fn raw(base: &str, edits: &[Edit]) -> String {
    let refs: Vec<&Edit> = edits.iter().collect();
    edit::spliced(base, &refs)
}

// Which blocks do not parse on their own, one trial each: the report a refusal of the whole text cannot give by itself. A trial that is not a verdict has nothing to say about its block.
fn blame(
    base: &str,
    edits: &[Edit],
    format: Format,
    path: &Path,
    stop: &Arc<Stop>,
) -> Option<Vec<(usize, String)>> {
    let mut out = Vec::new();
    for (at, edit) in edits.iter().enumerate() {
        let picked = [edit];
        match format(&edit::spliced(base, &picked), path, stop) {
            Outcome::Unparsed(why) => out.push((at, why.message)),
            Outcome::Stopped => return None,
            Outcome::Formatted(_) | Outcome::Undecided(_) => {}
        }
    }
    Some(out)
}

// Where each change's own lines ended up, in the order the changes were handed over. A marker comment takes no line number of its own, so its neighbours are the exact lines the change became. Positions count the text without the marker lines; `unmarked` reads them in the text that is written.
fn marks(text: &str, tag: &str) -> Vec<edit::Place> {
    let mut blocks: Vec<edit::Place> = Vec::new();
    let mut open: Option<(usize, usize)> = None;
    let mut at = 0usize;
    for line in text.lines() {
        let Some((index, edge)) = mark_at(line, tag) else {
            at += 1;
            continue;
        };
        match edge {
            Edge::Open => open = Some((index, at)),
            Edge::Close => {
                if let Some((index, first)) = open.take() {
                    if blocks.len() <= index {
                        blocks.resize(index + 1, None);
                    }
                    blocks[index] = Some((first + 1, at));
                }
            }
        }
    }
    blocks
}

// The formatted text without its marker lines, and how many lines before each one it lost. A marker is a comment, and a comment keeps a formatter from folding the blank lines around it; with the markers gone, a run of blank lines is folded to one, as the formatter would have done. A line a landing holds is never folded, so no block loses its own edges. The count says how many lines were dropped above each line of the marked text, so a landing found there can be read in the text that is written.
fn unmarked(text: &str, tag: &str, landings: &[edit::Place]) -> (String, Vec<usize>) {
    let stripped: Vec<&str> = text
        .lines()
        .filter(|line| mark_at(line, tag).is_none())
        .collect();
    let mut held = vec![false; stripped.len() + 2];
    for (first, last) in landings.iter().flatten() {
        for line in *first..=*last {
            if let Some(slot) = held.get_mut(line) {
                *slot = true;
            }
        }
    }
    let mut kept: Vec<&str> = Vec::with_capacity(stripped.len());
    // `dropped[i]` = lines lost above the marked text's line i, 1-based.
    let mut dropped = vec![0usize; stripped.len() + 2];
    let mut lost = 0usize;
    let mut blank = false;
    for (index, line) in stripped.iter().enumerate() {
        dropped[index + 1] = lost;
        let is_blank = line.trim().is_empty();
        if is_blank && blank && !held[index + 1] {
            lost += 1;
            continue;
        }
        blank = is_blank;
        kept.push(line);
    }
    dropped[stripped.len() + 1] = lost;
    let mut out = kept.join("\n");
    if text.ends_with('\n') && !out.is_empty() {
        out.push('\n');
    }
    (out, dropped)
}

// One landing, read in the written text: both ends lose the lines dropped above them.
fn placed(first: usize, last: usize, dropped: &[usize]) -> (usize, usize) {
    let at = |line: usize| line.saturating_sub(dropped.get(line).copied().unwrap_or(0));
    if last >= first {
        (at(first), at(last))
    } else {
        let spot = at(first);
        (spot, spot - 1)
    }
}

// One edge of a change's comment.
enum Edge {
    Open,
    Close,
}

// This run's marker word: the constant, and a fingerprint of the snapshot it marks.
fn tag(base: &str) -> String {
    let mut hasher = DefaultHasher::new();
    base.hash(&mut hasher);
    format!("{}:{:016x}", MARK, hasher.finish())
}

// The change index and edge this line carries, when it is one of the wrapper comments.
fn mark_at(line: &str, tag: &str) -> Option<(usize, Edge)> {
    let trimmed = line.trim();
    let rest = trimmed.strip_prefix("//")?.trim_start();
    let mut words = rest.split_whitespace();
    if words.next()? != tag {
        return None;
    }
    let index = words.next()?.parse().ok()?;
    let edge = match words.next()? {
        "open" => Edge::Open,
        "close" => Edge::Close,
        _ => return None,
    };
    Some((index, edge))
}
