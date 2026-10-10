//! Round open: user text in, a round running. The driver implements this: it lands the message,
//! opens the loop, and closes the round when the loop stops.

use anyhow::Result;

use super::Opened;

/// Contract of the round-opening door: a session id and a text already split into prefix / body /
/// suffix. `Opened::Busy` means try again after the round ends.
pub trait SubmitOp {
    /// Saying whether the round started is all this answers; the pieces arrive on the stream.
    fn submit(
        &mut self,
        session: i64,
        prefix: Option<&str>,
        text: &str,
        suffix: Option<&str>,
    ) -> Result<Opened>;
}
