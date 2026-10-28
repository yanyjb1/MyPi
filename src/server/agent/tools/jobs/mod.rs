//! A session's background jobs: schedule one, wake later, hand the output back once.

mod cron;
mod duration;
mod job;
mod output;
mod runner;
mod scheduler;
mod wake;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use anyhow::{Result, bail};

use crate::server::config::KairosConfig;
use crate::server::platform::time;

pub(crate) use cron::Cron;
pub(crate) use duration::parse as parse_duration;
pub(crate) use job::{Id, JobSpec, Receipt, Row, Schedule, Status, TaskState};
pub(crate) use output::OutputFile;
pub(crate) use runner::{JobCtx, JobOutcome, JobRunner};
pub(crate) use wake::{Wake, WakeSink};

use job::{Job, ScheduleEntry, State};
use output::OutputDir;

/// Builds one service per session and owns the shared wake handle.
pub(crate) struct JobFactory {
    cfg: KairosConfig,
    wake: Arc<dyn WakeSink>,
}

impl JobFactory {
    pub(crate) fn new(cfg: KairosConfig, wake: Arc<dyn WakeSink>) -> Self {
        OutputDir::clean_root();
        Self { cfg, wake }
    }

    pub(crate) fn make(&self, session: i64) -> Arc<JobService> {
        let dir = OutputDir::new(session, self.cfg.max_output_bytes, self.cfg.max_retained);
        let state = Arc::new(Mutex::new(State::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let bell = Arc::new((Mutex::new(()), Condvar::new()));
        scheduler::spawn(
            session,
            self.cfg,
            self.wake.clone(),
            state.clone(),
            stop.clone(),
            bell.clone(),
            dir.clone(),
        );
        Arc::new(JobService {
            inner: Arc::new(Inner {
                session,
                cfg: self.cfg,
                wake: self.wake.clone(),
                state,
                stop,
                bell,
                dir,
            }),
        })
    }

    pub(crate) fn sink(&self) -> Arc<dyn WakeSink> {
        self.wake.clone()
    }
}

/// The handle tools and the API hold.
pub struct JobService {
    inner: Arc<Inner>,
}

struct Inner {
    session: i64,
    cfg: KairosConfig,
    wake: Arc<dyn WakeSink>,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    bell: Arc<(Mutex<()>, Condvar)>,
    dir: OutputDir,
}

impl Inner {
    fn cancel_all(&self) {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        for task in st.tasks.values_mut() {
            task.cancel.store(true, Ordering::Relaxed);
            task.wake_on_done = false;
            task.state = TaskState::Canceled;
            if let Some(output) = task.output.take() {
                output.discard();
            }
        }
        st.tasks.clear();
        st.schedules.clear();
        st.running = 0;
        st.pending_wakes.clear();
        drop(st);
        self.dir.cleanup();
        self.bell.1.notify_all();
    }
}

impl Drop for Inner {
    fn drop(&mut self) {
        self.cancel_all();
        self.stop.store(true, Ordering::Relaxed);
        self.bell.1.notify_all();
    }
}

impl JobService {
    pub(crate) fn start(&self, spec: JobSpec) -> Result<Receipt> {
        let spec = self.fill(spec);
        let now = time::unix_now();
        match spec.schedule.clone() {
            Schedule::Now => self.now(spec, now),
            Schedule::After(delay) => self.after(spec, delay, now),
            Schedule::Every(cron, count) => self.every(spec, cron, count, now),
        }
    }

    fn now(&self, spec: JobSpec, now: i64) -> Result<Receipt> {
        let mut st = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        if spec.needs_slot {
            anyhow::ensure!(
                st.running < self.inner.cfg.max_concurrent,
                "the job pool is full"
            );
        }
        let id = st.task_id();
        let dir = self.inner.dir.task_dir(id);
        let output = if spec.output {
            Some(self.inner.dir.create(id)?)
        } else {
            None
        };
        let mut task = Job::new(id, spec, dir, output, None);
        task.state = TaskState::Running;
        task.started = Some(now);
        if task.needs_slot {
            st.running += 1;
        }
        st.tasks.insert(id, task);
        drop(st);
        scheduler::run_job(
            self.inner.session,
            self.inner.state.clone(),
            self.inner.bell.clone(),
            id,
        );
        Ok(Receipt {
            id,
            text: format!("{id} started"),
        })
    }

    fn after(&self, spec: JobSpec, delay: std::time::Duration, now: i64) -> Result<Receipt> {
        let mut st = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let id = st.task_id();
        let dir = self.inner.dir.task_dir(id);
        let output = if spec.output {
            Some(self.inner.dir.create(id)?)
        } else {
            None
        };
        let at = now.saturating_add(delay.as_secs() as i64);
        let task = Job::new(id, spec, dir, output, Some(at));
        st.tasks.insert(id, task);
        drop(st);
        self.inner.bell.1.notify_all();
        Ok(Receipt {
            id,
            text: format!("{id} scheduled for {}", stamp(at)),
        })
    }

    fn every(&self, spec: JobSpec, cron: Cron, count: Option<u32>, now: i64) -> Result<Receipt> {
        anyhow::ensure!(count != Some(0), "`count` must be positive or -1");
        let mut st = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let sid = st.schedule_id();
        let next = cron
            .next(now)
            .ok_or_else(|| anyhow::anyhow!("cron has no next trigger"))?;
        st.schedules.insert(
            sid,
            ScheduleEntry {
                id: sid,
                cron,
                spec,
                remaining: count,
                next: Some(next),
                skipped: 0,
                running: None,
                runs: std::collections::VecDeque::new(),
                next_run: 0,
            },
        );
        drop(st);
        self.inner.bell.1.notify_all();
        Ok(Receipt {
            id: Id::Schedule(sid),
            text: format!("s{sid} next {}", stamp(next)),
        })
    }

    /// Make a job now, but do not start it until `activate`.
    pub(crate) fn reserve(&self, spec: JobSpec) -> Result<Receipt> {
        let spec = self.fill(spec);
        let mut st = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let id = st.task_id();
        let dir = self.inner.dir.task_dir(id);
        let output = if spec.output {
            Some(self.inner.dir.create(id)?)
        } else {
            None
        };
        let task = Job::new(id, spec, dir, output, None);
        st.tasks.insert(id, task);
        drop(st);
        Ok(Receipt {
            id,
            text: format!("{id} reserved"),
        })
    }

    /// Let a reserved job become due.
    pub(crate) fn activate(&self, id: Id) -> Result<()> {
        let mut st = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let task = st
            .tasks
            .get_mut(&id)
            .ok_or_else(|| anyhow::anyhow!("no job {id}"))?;
        anyhow::ensure!(
            task.state == TaskState::Scheduled && task.start_at.is_none(),
            "{id} is not reserved"
        );
        task.start_at = Some(time::unix_now());
        drop(st);
        self.inner.bell.1.notify_all();
        Ok(())
    }

    pub(crate) fn list(&self) -> Vec<Row> {
        let st = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut rows = Vec::new();
        for task in st.tasks.values() {
            rows.push(Row {
                id: task.id,
                kind: task.kind.clone(),
                state: task.state.to_string(),
                bytes: task.bytes,
            });
        }
        for schedule in st.schedules.values() {
            rows.push(Row {
                id: Id::Schedule(schedule.id),
                kind: schedule.spec.kind.to_string(),
                state: "scheduled".to_string(),
                bytes: 0,
            });
        }
        rows
    }

    pub(crate) fn status(&self, id: Id) -> Result<Status> {
        let st = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Id::Schedule(sid) = id {
            let schedule = st
                .schedules
                .get(&sid)
                .ok_or_else(|| anyhow::anyhow!("no job schedule {id}"))?;
            return Ok(Status {
                id,
                state: "scheduled".to_string(),
                exit: None,
                duration: 0,
                bytes: 0,
                truncated: false,
                next: schedule.next,
                remaining: schedule.remaining,
                skipped: schedule.skipped,
            });
        }
        let task = st
            .tasks
            .get(&id)
            .ok_or_else(|| anyhow::anyhow!("no job {id}"))?;
        Ok(Status {
            id,
            state: task.state.to_string(),
            exit: task.exit,
            duration: task.duration(),
            bytes: task.bytes,
            truncated: task.truncated,
            next: None,
            remaining: None,
            skipped: 0,
        })
    }

    pub(crate) fn result(&self, id: Id) -> Result<String> {
        if matches!(id, Id::Schedule(_)) {
            bail!("{id} is a schedule; ask for one of its runs");
        }
        let (text, output) = {
            let mut st = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
            let task = st
                .tasks
                .get_mut(&id)
                .ok_or_else(|| anyhow::anyhow!("no job {id}"))?;
            match task.state {
                TaskState::Running | TaskState::Scheduled | TaskState::Waiting => {
                    bail!("{id} is still running")
                }
                TaskState::Expired => bail!("{id} expired"),
                TaskState::Done | TaskState::Failed | TaskState::Canceled => {}
            }
            if task.consumed {
                bail!("{id} was already consumed");
            }
            task.consumed = true;
            (task.text.clone(), task.output.take())
        };
        let text = match output {
            Some(output) => output.read_and_remove()?,
            None => text.unwrap_or_default(),
        };
        self.inner.dir.remove_task_dir(id);
        Ok(text)
    }

    pub(crate) fn cancel(&self, id: Id, notify: bool) -> Result<String> {
        let mut st = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        match id {
            Id::Schedule(sid) => {
                anyhow::ensure!(st.schedules.remove(&sid).is_some(), "no job schedule {id}");
            }
            Id::Task(_) | Id::Run(_, _) => {
                let task = st
                    .tasks
                    .get_mut(&id)
                    .ok_or_else(|| anyhow::anyhow!("no job {id}"))?;
                if matches!(
                    task.state,
                    TaskState::Done | TaskState::Failed | TaskState::Canceled | TaskState::Expired
                ) {
                    bail!("{id} is already {}", task.state);
                }
                task.cancel.store(true, Ordering::Relaxed);
                task.wake_on_done = false;
                task.state = TaskState::Canceled;
                task.ended = Some(time::unix_now());
            }
        }
        if notify {
            st.pending_wakes.push(wake::canceled(id));
        }
        drop(st);
        self.inner.bell.1.notify_all();
        Ok(format!("{id} canceled"))
    }

    pub(crate) fn cancel_all(&self) {
        self.inner.cancel_all();
    }

    fn fill(&self, mut spec: JobSpec) -> JobSpec {
        if spec.timeout.is_none() {
            spec.timeout = Some(std::time::Duration::from_secs(self.inner.cfg.timeout_secs));
        }
        spec
    }

    pub(crate) fn session(&self) -> i64 {
        self.inner.session
    }

    pub(crate) fn sink(&self) -> Arc<dyn WakeSink> {
        self.inner.wake.clone()
    }
}

fn stamp(unix: i64) -> String {
    let (y, m, d, h, min, s) = time::local_parts(unix);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{min:02}:{s:02}")
}
