//! The shapes one session's scheduler keeps in memory.

use std::collections::{BTreeMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use super::cron::Cron;
use super::output::OutputFile;
use super::runner::JobRunner;

/// A job (`k1`), a schedule (`s1`), or one schedule run (`s1.1`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub(crate) enum Id {
    Task(u64),
    Schedule(u64),
    Run(u64, u32),
}

impl Id {
    pub(crate) fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if let Some(rest) = text.strip_prefix('k') {
            return Some(Id::Task(rest.parse().ok()?));
        }
        let rest = text.strip_prefix('s')?;
        if let Some((schedule, run)) = rest.split_once('.') {
            return Some(Id::Run(schedule.parse().ok()?, run.parse().ok()?));
        }
        Some(Id::Schedule(rest.parse().ok()?))
    }
}

impl std::fmt::Display for Id {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Id::Task(n) => write!(f, "k{n}"),
            Id::Schedule(n) => write!(f, "s{n}"),
            Id::Run(s, n) => write!(f, "s{s}.{n}"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TaskState {
    Scheduled,
    Waiting,
    Running,
    Done,
    Failed,
    Canceled,
    Expired,
}

impl TaskState {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            TaskState::Scheduled => "scheduled",
            TaskState::Waiting => "waiting",
            TaskState::Running => "running",
            TaskState::Done => "done",
            TaskState::Failed => "failed",
            TaskState::Canceled => "canceled",
            TaskState::Expired => "expired",
        }
    }
}

impl std::fmt::Display for TaskState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Clone)]
pub(crate) enum Schedule {
    Now,
    After(std::time::Duration),
    Every(Cron, Option<u32>),
}

#[derive(Clone)]
pub(crate) struct JobSpec {
    pub(crate) runner: Arc<dyn JobRunner>,
    pub(crate) schedule: Schedule,
    pub(crate) timeout: Option<std::time::Duration>,
    pub(crate) output: bool,
    pub(crate) needs_slot: bool,
    pub(crate) kind: &'static str,
    pub(crate) wake_on_done: bool,
}

pub(crate) struct Job {
    pub(crate) id: Id,
    pub(crate) runner: Option<Arc<dyn JobRunner>>,
    pub(crate) kind: String,
    pub(crate) timeout: std::time::Duration,
    pub(crate) output: Option<OutputFile>,
    pub(crate) state: TaskState,
    pub(crate) started: Option<i64>,
    pub(crate) ended: Option<i64>,
    pub(crate) exit: Option<i32>,
    pub(crate) bytes: usize,
    pub(crate) truncated: bool,
    pub(crate) cancel: Arc<AtomicBool>,
    pub(crate) consumed: bool,
    pub(crate) dir: std::path::PathBuf,
    pub(crate) start_at: Option<i64>,
    pub(crate) schedule: Option<u64>,
    pub(crate) wake_on_done: bool,
    pub(crate) needs_slot: bool,
    pub(crate) text: Option<String>,
}

impl Job {
    pub(crate) fn new(
        id: Id,
        spec: JobSpec,
        dir: std::path::PathBuf,
        output: Option<OutputFile>,
        start_at: Option<i64>,
    ) -> Self {
        Self {
            id,
            runner: Some(spec.runner),
            kind: spec.kind.to_string(),
            timeout: spec.timeout.unwrap_or_default(),
            output,
            state: TaskState::Scheduled,
            started: None,
            ended: None,
            exit: None,
            bytes: 0,
            truncated: false,
            cancel: Arc::new(AtomicBool::new(false)),
            consumed: false,
            dir,
            start_at,
            schedule: None,
            wake_on_done: spec.wake_on_done,
            needs_slot: spec.needs_slot,
            text: None,
        }
    }

    pub(crate) fn duration(&self) -> u64 {
        match (self.started, self.ended) {
            (Some(start), Some(end)) => end.saturating_sub(start).max(0) as u64,
            _ => 0,
        }
    }
}

pub(crate) struct ScheduleEntry {
    pub(crate) id: u64,
    pub(crate) cron: Cron,
    pub(crate) spec: JobSpec,
    pub(crate) remaining: Option<u32>,
    pub(crate) next: Option<i64>,
    pub(crate) skipped: u32,
    pub(crate) running: Option<Id>,
    pub(crate) runs: VecDeque<Id>,
    pub(crate) next_run: u32,
}

pub(crate) struct Receipt {
    pub(crate) id: Id,
    pub(crate) text: String,
}

pub(crate) struct Row {
    pub(crate) id: Id,
    pub(crate) kind: String,
    pub(crate) state: String,
    pub(crate) bytes: usize,
}

pub(crate) struct Status {
    pub(crate) id: Id,
    pub(crate) state: String,
    pub(crate) exit: Option<i32>,
    pub(crate) duration: u64,
    pub(crate) bytes: usize,
    pub(crate) truncated: bool,
    pub(crate) next: Option<i64>,
    pub(crate) remaining: Option<u32>,
    pub(crate) skipped: u32,
}

#[derive(Default)]
pub(crate) struct State {
    pub(crate) tasks: BTreeMap<Id, Job>,
    pub(crate) schedules: BTreeMap<u64, ScheduleEntry>,
    pub(crate) running: usize,
    pub(crate) next_task: u64,
    pub(crate) next_schedule: u64,
    pub(crate) pending_wakes: Vec<String>,
}

impl State {
    pub(crate) fn task_id(&mut self) -> Id {
        self.next_task += 1;
        Id::Task(self.next_task)
    }

    pub(crate) fn schedule_id(&mut self) -> u64 {
        self.next_schedule += 1;
        self.next_schedule
    }
}
