//! The agent job runner: run one child, then settle its final file.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;

use crate::server::agent::tools::jobs::{JobCtx, JobOutcome, JobRunner};

use super::draft::Drafts;
use super::spawn::Spawn;

pub(crate) struct AgentRunner {
    child: i64,
    root: PathBuf,
    spawn: Arc<dyn Spawn>,
    drafts: Drafts,
}

impl AgentRunner {
    pub(crate) fn new(child: i64, root: PathBuf, spawn: Arc<dyn Spawn>, drafts: Drafts) -> Self {
        Self {
            child,
            root,
            spawn,
            drafts,
        }
    }
}

impl JobRunner for AgentRunner {
    fn run(&self, ctx: &mut JobCtx<'_>) -> Result<JobOutcome> {
        let fallback = self.spawn.run(self.child, ctx.cancel.clone())?;
        let ok = self
            .drafts
            .finish(&self.root, self.child, fallback.as_deref())?;
        Ok(JobOutcome {
            ok,
            text: None,
            wake: None,
            exit: None,
        })
    }
}
