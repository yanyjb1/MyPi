//! The only extension point between the scheduler and one kind of work.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use anyhow::Result;

use super::job::Id;
use super::output::OutputFile;

/// One job's run-time view: what it may read, and where it may write.
pub(crate) struct JobCtx<'a> {
    pub(crate) session: i64,
    pub(crate) job: Id,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) dir: PathBuf,
    pub(crate) timeout: std::time::Duration,
    pub(crate) output: Option<&'a mut OutputFile>,
}

/// What one run reports back.
pub(crate) struct JobOutcome {
    pub(crate) ok: bool,
    pub(crate) text: Option<String>,
    pub(crate) wake: Option<String>,
    pub(crate) exit: Option<i32>,
}

impl JobOutcome {
    pub(crate) fn ok() -> Self {
        Self {
            ok: true,
            text: None,
            wake: None,
            exit: None,
        }
    }

    pub(crate) fn failed() -> Self {
        Self {
            ok: false,
            text: None,
            wake: None,
            exit: None,
        }
    }
}

pub(crate) trait JobRunner: Send + Sync {
    fn run(&self, ctx: &mut JobCtx<'_>) -> Result<JobOutcome>;
}
