//! A text job: nothing to run, so it only ever becomes due on the clock.

use anyhow::Result;

use crate::server::agent::tools::contract::job::{JobCtx, JobOutcome, JobRunner};

/// Wakes the session with the text it was handed.
pub(crate) struct TextRunner {
    pub(crate) text: String,
}

impl JobRunner for TextRunner {
    fn run(&self, _ctx: &mut JobCtx<'_>) -> Result<JobOutcome> {
        Ok(JobOutcome {
            ok: true,
            text: Some(self.text.clone()),
            wake: Some(self.text.clone()),
            exit: None,
        })
    }
}
