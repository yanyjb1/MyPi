//! The state that outlives one turn.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use super::Transport;
use super::book::Birth;
use crate::server::agent::request::message::ToolDef;
use crate::server::agent::tools::contract::call::Cwd;

/// One session's live state.
pub struct Live {
    pub provider: String,
    pub model: String,
    pub system: String,
    /// The session's tool names; the front door's gate.
    pub tools: Vec<String>,
    /// The manuals built when the session was booked; the request prefix reads these, never a rebuild.
    pub manuals: Arc<Vec<ToolDef>>,
    /// Those manuals as the wire body wants them, serialized once with the session.
    pub tools_json: Arc<serde_json::Value>,
    /// The compaction instruction template; it comes from the profile the caller resolved.
    pub compact: String,
    /// The launch directory the front end handed `init`; this process never moves it.
    pub root: PathBuf,
    /// The working directory handle: `cd` writes it, every batch reads it.
    pub cwd: Cwd,
    /// Where full texts are exported on demand.
    pub dir: PathBuf,
}

impl Live {
    pub fn new(birth: &Birth, dir: &Path) -> Self {
        // The launch directory comes from the front end; this half never guesses one.
        let root = birth.cwd.clone();
        let tools_json =
            serde_json::to_value(birth.manuals.as_ref()).expect("tool manuals serialize to JSON");
        Self {
            provider: birth.provider.clone(),
            model: birth.model.clone(),
            system: birth.system.clone(),
            tools: birth.tools.clone(),
            manuals: birth.manuals.clone(),
            tools_json: Arc::new(tools_json),
            compact: birth.compact.clone(),
            root: root.clone(),
            cwd: Arc::new(RwLock::new(root)),
            dir: dir.to_path_buf(),
        }
    }

    pub fn transport(&self) -> Transport {
        Transport::new(self.provider.clone(), self.model.clone())
    }
}
