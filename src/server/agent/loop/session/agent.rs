//! The sub-agent job: run one child session to its end, then settle it.

use std::path::PathBuf;
use std::sync::Weak;
use std::sync::atomic::Ordering;
use std::time::Instant;

use anyhow::Result;

use super::book::{Birth, Inner, book_child, unbook};
use crate::server::agent::tools::contract::job::{JobCtx, JobOutcome, JobRunner};
use crate::server::agent::tools::contract::view::Source;

/// One child session, held open until `final submit` or until the nags run out.
pub(crate) struct AgentRunner {
    inner: Weak<Inner>,
    child: i64,
    birth: Birth,
    start: String,
    answer: PathBuf,
}

impl AgentRunner {
    pub(crate) fn new(
        inner: Weak<Inner>,
        child: i64,
        birth: Birth,
        start: String,
        answer: PathBuf,
    ) -> Self {
        Self {
            inner,
            child,
            birth,
            start,
            answer,
        }
    }

    /// The answer file's stem: the name the parent was told at spawn.
    fn name(&self) -> String {
        self.answer
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_default()
    }
}

impl JobRunner for AgentRunner {
    fn run(&self, ctx: &mut JobCtx<'_>) -> Result<JobOutcome> {
        let name = self.name();
        let Some(inner) = self.inner.upgrade() else {
            return Ok(JobOutcome {
                ok: false,
                text: None,
                wake: Some(format!("{name} failed: the loop is gone")),
                exit: None,
            });
        };
        let handle = match book_child(&inner, self.birth.clone(), self.start.clone()) {
            Ok(handle) => handle,
            Err(e) => {
                return Ok(JobOutcome {
                    ok: false,
                    text: None,
                    wake: Some(format!("{name} failed to start: {e:#}")),
                    exit: None,
                });
            }
        };
        let deadline = Instant::now() + ctx.timeout;
        // A cancellation rings the same bell the session rings when its turn ends.
        let hook = {
            let handle = handle.clone();
            ctx.cancel.on_stop(move || handle.ring_settle())
        };
        let mut stopped = false;
        loop {
            if !handle.busy.load(Ordering::Acquire) {
                break;
            }
            let now = Instant::now();
            if !stopped && (ctx.cancel.is_stopped() || now >= deadline) {
                stopped = true;
                handle.interrupt();
                handle.wait_idle();
                break;
            }
            handle.wait_idle_for(deadline.saturating_duration_since(now));
        }
        drop(hook);
        let submitted = inner
            .source
            .child(self.child)
            .is_some_and(|child| child.submitted());
        unbook(&inner, self.child);
        let ok = submitted && !stopped;
        let wake = if ok {
            format!("{name} finished: {}", self.answer.display())
        } else if stopped {
            format!("{name} failed: the task was stopped")
        } else {
            format!("{name} failed: no final submit")
        };
        Ok(JobOutcome {
            ok,
            text: Some(self.answer.display().to_string()),
            wake: Some(wake),
            exit: None,
        })
    }
}
