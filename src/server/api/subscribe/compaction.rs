//! `/compact`: the door that opens a summarization of a context region. The machinery is the request
//! side's; this takes the parsed retention and runs once.

use anyhow::Result;

use super::Opened;

/// How much of the transcript tail to keep verbatim, as `/compact`'s argument asked for. `Default` =
/// the user gave no budget, so the config's `retainTail` is used.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Retention {
    /// No argument: the configured tail budget.
    Default,
    /// A token budget. Whole blocks are claimed newest-first while they fit.
    Tokens(usize),
    /// A fixed number of newest blocks, whatever they weigh. The escape
    /// hatch for a transcript whose blocks are unusually fat or thin.
    Blocks(usize),
}

/// Contract of the compaction door. A refusal or a nothing-to-do is an
/// `Opened::Busy`, never an error; a failure to run at all is an error.
pub trait CompactOp {
    /// `focus` is the free text after `/compact`'s number. The answer says whether the compaction
    /// started; its pieces arrive on the session's stream.
    fn compact(
        &mut self,
        session: i64,
        retention: Retention,
        focus: Option<String>,
    ) -> Result<Opened>;
}
