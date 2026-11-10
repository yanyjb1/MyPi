//! The job vocabulary a tool shares with the loop: what to run, when it is due, what came back.

use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;

/// One task, as the caller describes it.
pub struct JobSpec {
    pub runner: Arc<dyn JobRunner>,
    pub schedule: Schedule,
    pub timeout: Option<Duration>,
    /// Keep the run's output in a file for `result`.
    pub output: bool,
    /// Take a place in the session's pool while it runs.
    pub needs_slot: bool,
    pub kind: &'static str,
    /// Wake the session once the run is over.
    pub wake_on_done: bool,
}

/// When a task becomes due: at once, after a delay, or on a cron expression.
pub enum Schedule {
    Now,
    After(Duration),
    Every { expr: String, count: Option<u32> },
}

/// What one kind of work does.
pub trait JobRunner: Send + Sync {
    fn run(&self, ctx: &mut JobCtx<'_>) -> Result<JobOutcome>;
}

/// One run's conditions.
pub struct JobCtx<'a> {
    pub cancel: Arc<crate::server::platform::stop::Stop>,
    pub timeout: Duration,
    pub output: Option<&'a mut dyn OutputSink>,
}

/// What one run reports back.
pub struct JobOutcome {
    pub ok: bool,
    /// The text `result` hands out when the run kept no output file.
    pub text: Option<String>,
    /// The wake text; absent means the loop writes its own from the state.
    pub wake: Option<String>,
    pub exit: Option<i32>,
}

impl JobOutcome {
    pub fn failed() -> Self {
        Self {
            ok: false,
            text: None,
            wake: None,
            exit: None,
        }
    }
}

/// What starting a task answers; its id is what the caller holds.
pub struct Receipt {
    pub id: String,
    pub text: String,
    /// The next scheduled local time, absent when the task starts at once.
    pub at: Option<String>,
}

/// One row of the session's task list.
pub struct Row {
    pub id: String,
    pub kind: String,
    pub state: String,
    pub bytes: usize,
}

/// One task's condition.
pub struct Status {
    pub id: String,
    pub state: String,
    pub exit: Option<i32>,
    pub duration: u64,
    pub bytes: usize,
    pub truncated: bool,
    pub next: Option<i64>,
    pub remaining: Option<u32>,
    pub skipped: u32,
}

/// The handle a tool holds: five actions, nothing else.
pub trait JobHost: Send + Sync {
    fn start(&self, spec: JobSpec) -> Result<Receipt>;
    fn list(&self) -> Vec<Row>;
    fn status(&self, id: &str) -> Result<Status>;
    fn result(&self, id: &str) -> Result<String>;
    fn cancel(&self, id: &str) -> Result<String>;
}

/// Where one command's output goes while it runs.
pub trait OutputSink {
    fn out(&mut self, chunk: &str) -> Result<()>;
    fn err(&mut self, chunk: &str) -> Result<()>;
    fn done(&mut self, exit: &Exit) -> Result<()>;
}

/// Why a command stopped. `canceled` means an external stop armed the flag before the deadline.
pub struct Exit {
    pub code: Option<i32>,
    pub killed: bool,
    pub canceled: bool,
    pub timeout_secs: u64,
}
