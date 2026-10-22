//! Everything that happens without the user: one flat signal enum.
//!
//! Two sources. Runtime events are ours to name; environment events are the terminal's,
//! which we can only read. A signal carries no data: whoever needs the details reads them
//! from the session view.

/// Something happened. No payload, no meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    // -- runtime --
    /// A round opened. `Blocks` starts arriving after this.
    RoundStarted,
    /// A round closed. The last landed batch may still arrive after it.
    RoundEnded,
    /// A batch of blocks arrived. Mid-round only: between `RoundStarted` and the last
    /// landed batch. Never fires outside a round.
    Blocks,
    /// A tool call started running.
    ToolStarted,
    /// A tool call finished.
    ToolFinished,
    /// A line for the human arrived (a notice).
    Notice,
    /// An idle frame: nothing new, something on screen still moves.
    Tick,
    /// A deadline we set earlier came due.
    Deadline,
    /// The session connection dropped.
    Disconnected,
    /// The session connection is back.
    Reconnected,
    // -- environment --
    /// The terminal changed size.
    Resized,
    /// The process was suspended.
    Suspended,
    /// The process was resumed.
    Resumed,
    /// The terminal window took focus.
    FocusGained,
    /// The terminal window lost focus.
    FocusLost,
    /// The terminal is gone (SIGHUP / SIGTERM).
    TerminalGone,
    /// Writing to the terminal failed.
    OutputBroken,
}
