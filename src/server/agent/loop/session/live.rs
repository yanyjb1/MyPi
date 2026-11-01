//! The state that outlives one turn.

use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use super::Transport;
use super::book::Birth;
use crate::server::agent::tools::contract::Cwd;

/// One session's live state.
pub struct Live {
    pub provider: String,
    pub model: String,
    pub system: String,
    /// The session's tool names; the manuals are expanded per request.
    pub tools: Vec<String>,
    /// The compaction instruction template; it comes from the profile the caller resolved.
    pub compact: String,
    /// Where the session started; it never moves.
    pub root: PathBuf,
    /// The working directory handle: `cd` writes it, every batch reads it.
    pub cwd: Cwd,
    /// Where full texts are exported on demand.
    pub dir: PathBuf,
}

impl Live {
    pub fn new(birth: &Birth, dir: &Path) -> Self {
        let root = birth
            .cwd
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .unwrap_or_default();
        Self {
            provider: birth.provider.clone(),
            model: birth.model.clone(),
            system: birth.system.clone(),
            tools: birth.tools.clone(),
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
