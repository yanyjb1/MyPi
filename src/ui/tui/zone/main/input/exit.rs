//! What the editor declares when it cannot act on its own.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitRequest {
    /// Submit the current input, whole.
    Submit(String),
    /// Interrupt the in-flight round.
    Interrupt,
    /// Two un-lent Esc inside the window ask for the node tree.
    OpenTree,
    /// `/resume`: the picker is the app's to show.
    OpenResume,
    /// Quit the program.
    Quit,
    /// `/new`: reset the conversation without attaching another.
    New,
}
