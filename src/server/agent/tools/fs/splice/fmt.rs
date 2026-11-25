// The language's formatter as one pass over the merged text. The text carries the runs this batch wrote, and the language hands back where each run landed; nothing here knows how a run is marked, or how a language lays a blank line out.

use std::path::Path;
use std::sync::Arc;

use super::edit::{self, Edit};

use crate::server::agent::tools::fs::lang::Format;
use crate::server::agent::tools::fs::lang::verdict::{Draft, Outcome};
use crate::server::platform::stop::Stop;

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

// Every edit spliced into the snapshot once, handed to the language as one text with the runs this batch wrote. The language answers the text it laid out and where each run landed; a text it cannot parse is written as it stands, and each block is then tried on its own only to say which one it blames. `None` = the turn was stopped, so nothing is written.
pub(crate) fn settle(
    base: &str,
    edits: &[Edit],
    format: Format,
    path: &Path,
    stop: &Arc<Stop>,
) -> Option<Settled> {
    let drafted = raw(base, edits);
    let runs = spells(edits);
    match format(
        &Draft {
            text: &drafted,
            runs: &runs,
        },
        path,
        stop,
    ) {
        Outcome::Formatted { text, runs } => Some(Settled {
            text,
            word: Word::Formatted,
            landings: runs.into_iter().collect(),
            blamed: Vec::new(),
        }),
        Outcome::Stopped => None,
        Outcome::Undecided(why) => Some(Settled {
            text: drafted,
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
                text: drafted,
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

// The runs the merged text holds, 1-based and in the order the changes were handed over: a change that left no line names the line after its own place, as a range that ends before it starts.
fn spells(edits: &[Edit]) -> Vec<(usize, usize)> {
    edit::shifted(edits)
        .into_iter()
        .map(|place| place.unwrap_or((1, 0)))
        .collect()
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
    for (at, change) in edits.iter().enumerate() {
        let picked = std::slice::from_ref(change);
        let drafted = raw(base, picked);
        let runs = spells(picked);
        match format(
            &Draft {
                text: &drafted,
                runs: &runs,
            },
            path,
            stop,
        ) {
            Outcome::Unparsed(why) => out.push((at, why.message)),
            Outcome::Stopped => return None,
            Outcome::Formatted { .. } | Outcome::Undecided(_) => {}
        }
    }
    Some(out)
}
