//! Create the child row, and the port the job runner uses to run it.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::{Context as _, Result};

use crate::server::storage::{Submit, Writer};

/// What the API must do to run one child to its end.
pub(crate) trait Spawn: Send + Sync {
    fn run(&self, session: i64, cancel: Arc<AtomicBool>) -> Result<Option<String>>;
}

/// What the child's `Live` needs beyond its own row.
#[derive(Clone)]
pub(crate) struct ChildSpec {
    pub(crate) parent: i64,
    pub(crate) parent_root: PathBuf,
    pub(crate) system: String,
    pub(crate) tools: Vec<String>,
    pub(crate) max_tool_calls: u32,
}

pub(crate) fn create(
    writer: &Writer,
    cwd: &str,
    model: Option<(String, String)>,
    profile: &str,
) -> Result<i64> {
    let landed = writer.push(Submit::NewSession {
        started_at: crate::server::platform::time::now_stamp(),
        cwd: cwd.to_string(),
        model,
        name: None,
        profile: Some(profile.to_string()),
    })?;
    landed
        .session
        .context("the writer created no child session")
}
