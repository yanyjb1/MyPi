//! What a zone asks the app to land on the session.

pub use crate::ui::session::{ModelId, Retention};

/// One ask. The zone says what it wants; the app names the door.
#[derive(Debug, Clone, PartialEq)]
pub enum Request {
    // Reads.
    Init,
    Meta,
    Usage,
    Pools,
    Config {
        key: String,
    },
    Blocks {
        ranges: Vec<[i64; 2]>,
    },
    Sessions {
        under: Option<String>,
    },
    // Writes.
    Submit(String),
    Interrupt,
    Mailbox {
        prefix: Option<String>,
        text: String,
        suffix: Option<String>,
    },
    ClearMailbox,
    Rename(Option<String>),
    SetCwd(String),
    SwitchModel(ModelId),
    SetDefaultModel(ModelId),
    Compact {
        retention: Option<Retention>,
        focus: Option<String>,
    },
    Delete(i64),
    Rewind {
        turn: i64,
        block: i64,
    },
    // The app's own.
    Attach(i64),
    Detach,
    Structure,
    Quit,
    /// The profile a session will open with; it lives in this front end until then.
    PickProfile(Option<String>),
}
