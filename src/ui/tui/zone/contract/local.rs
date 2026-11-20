// What a zone asks the front end itself: no letter reaches the server from here.

// One ask that never leaves this process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Local {
    // Follow one session; a `post init` answer names it.
    Attach(i64),
    Detach,
    Quit,
    // Text for the terminal's clipboard; the frame loop writes it between draws.
    Copy(String),
}
