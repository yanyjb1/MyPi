// The language's parser as one pass over the merged text. It answers whether the text parses, and which written block is at fault when it does not; it never lays the text out, so what lands is exactly what the batch spliced.

use std::path::Path;
use std::sync::Arc;

use super::edit::{self, Edit};

use crate::server::agent::tools::fs::lang::Format;
use crate::server::agent::tools::fs::lang::verdict::Outcome;
use crate::server::platform::stop::Stop;

// What the parser made of one merged change.
pub(crate) struct Settled {
    // The edits whose block the parser blames on its own, with its words. Positions into the edits handed over.
    pub blamed: Vec<(usize, String)>,
}

// Every edit spliced into the snapshot once, handed to the language as one text. A text the parser accepts leaves nothing to blame; a text it refuses is tried block by block, to say which one it blames. `None` = the turn was stopped, so nothing is written.
pub(crate) fn settle(
    base: &str,
    edits: &[Edit],
    format: Format,
    path: &Path,
    stop: &Arc<Stop>,
) -> Option<Settled> {
    let drafted = raw(base, edits);
    match format(&drafted, path, stop) {
        Outcome::Parsed => Some(Settled { blamed: Vec::new() }),
        Outcome::Stopped => None,
        Outcome::Unparsed(why) => {
            let blamed = match edits.len() {
                // One edit is the whole text: the parser has already named it.
                1 => vec![(0, why.message.clone())],
                _ => blame(base, edits, format, path, stop)?,
            };
            Some(Settled { blamed })
        }
    }
}

// The snapshot with every change in place: what is written, always.
fn raw(base: &str, edits: &[Edit]) -> String {
    let refs: Vec<&Edit> = edits.iter().collect();
    edit::spliced(base, &refs)
}

// Which blocks do not parse on their own, one trial each: the report a refusal of the whole text cannot give by itself.
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
        match format(&drafted, path, stop) {
            Outcome::Unparsed(why) => out.push((at, why.message)),
            Outcome::Stopped => return None,
            Outcome::Parsed => {}
        }
    }
    Some(out)
}
