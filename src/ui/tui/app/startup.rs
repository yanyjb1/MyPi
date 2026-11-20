//! External startup parameters. App reads only the fields it knows.

use std::path::PathBuf;

use crate::ui::tui::zone::ZoneId;

#[derive(Debug, Clone)]
pub struct Startup {
    /// `None` = Main.
    pub initial: Option<ZoneId>,

    /// A session to attach after the first frame; `None` = a draft.
    pub session: Option<i64>,

    /// The directory a picker or a draft starts in.
    pub cwd: PathBuf,

    /// `--model`: the draft's model address, `<provider>:<model>`; `None` = the configured default.
    pub model: Option<String>,
}

impl Default for Startup {
    fn default() -> Self {
        Self {
            initial: None,
            session: None,
            cwd: PathBuf::from("."),
            model: None,
        }
    }
}

impl Startup {
    pub fn main(cwd: PathBuf, model: Option<String>) -> Self {
        Self {
            cwd,
            model,
            ..Self::default()
        }
    }

    pub fn resume(cwd: PathBuf) -> Self {
        Self {
            initial: Some(ZoneId::Resume),
            session: None,
            cwd,
            model: None,
        }
    }

    pub fn attach(cwd: PathBuf, session: i64, model: Option<String>) -> Self {
        Self {
            initial: None,
            session: Some(session),
            cwd,
            model,
        }
    }
}
