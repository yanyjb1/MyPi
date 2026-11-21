// What the editor declares when it cannot act on its own.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExitRequest {
    // Two un-lent Esc inside the window ask for the node tree.
    OpenTree,

    // `/resume`: the picker is the app's to show.
    OpenResume,

    // `/hub`: this conversation's subagents are the app's to show.
    OpenHub,

    // Quit the program.
    Quit,

    // `/new`: reset the conversation without attaching another.
    New,
}
