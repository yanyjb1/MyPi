//! What the history zone asks the session for.

/// One report from the history zone. Landed by whoever owns the loop.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ask {
    /// Open a conversation: the structure and the newest blocks come back together.
    Init { session: i64 },
    /// Jump to a round. One round per ask, no direction.
    Jump { turn: i64 },
    /// Fill one side in: from this block, this way.
    Fetch { num: i64, older: bool },
    /// The reader stopped: the session may drop its throttling.
    Settled,
}
