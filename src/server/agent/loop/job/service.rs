//! The job service one session holds, and the factory that builds them.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

use anyhow::{Result, bail};

use super::cron::Cron;
use super::job::{Id, Job, ScheduleEntry, Spec, State, TaskState};
use super::output::OutputDir;
use super::scheduler;
use super::wake::{self, WakeSink};
use crate::server::agent::tools::contract::job as port;
use crate::server::platform::time;

/// The scheduler's own numbers, read from config by whoever builds the factory.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Values {
    pub(crate) max_concurrent: usize,
    pub(crate) timeout_secs: u64,
    pub(crate) max_output_bytes: usize,
    pub(crate) max_retained: usize,
}

/// Builds one service per session and owns the shared wake handle.
pub(crate) struct Factory {
    values: Values,
    wake: Arc<dyn WakeSink>,
}

impl Factory {
    pub(crate) fn new(values: Values, wake: Arc<dyn WakeSink>) -> Self {
        OutputDir::clean_root();
        Self { values, wake }
    }

    pub(crate) fn make(&self, session: i64) -> Arc<JobService> {
        let dir = OutputDir::new(
            session,
            self.values.max_output_bytes,
            self.values.max_retained,
        );
        let state = Arc::new(Mutex::new(State::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let bell = Arc::new((Mutex::new(()), Condvar::new()));
        scheduler::spawn(
            session,
            self.values,
            self.wake.clone(),
            state.clone(),
            stop.clone(),
            bell.clone(),
            dir.clone(),
        );
        Arc::new(JobService {
            inner: Arc::new(Inner {
                values: self.values,
                state,
                stop,
                bell,
                dir,
            }),
        })
    }
}

/// The handle one session's tools hold.
pub(crate) struct JobService {
    inner: Arc<Inner>,
}

struct Inner {
    values: Values,
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

impl port::JobHost for JobService {
    fn start(&self, spec: port::JobSpec) -> Result<port::Receipt> {
        let (spec, schedule) = Spec::take(spec);
        let spec = self.fill(spec);
        let now = time::unix_now();
        match schedule {
            port::Schedule::Now => self.now(spec, now),
            port::Schedule::After(delay) => self.after(spec, delay, now),
            port::Schedule::Every { expr, count } => {
                let cron = Cron::parse(&expr)?;
                self.every(spec, cron, count, now)
            }
        }
    }

    fn list(&self) -> Vec<port::Row> {
        let st = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut rows = Vec::new();
        for task in st.tasks.values() {
            rows.push(port::Row {
                id: task.id.to_string(),
                kind: task.kind.clone(),
                state: task.state.to_string(),
                bytes: task.bytes,
            });
        }
        for schedule in st.schedules.values() {
            rows.push(port::Row {
                id: Id::Schedule(schedule.id).to_string(),
                kind: schedule.spec.kind.to_string(),
                state: "scheduled".to_string(),
                bytes: 0,
            });
        }
        rows
    }

    fn status(&self, id: &str) -> Result<port::Status> {
        let id = parse_id(id)?;
        let st = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Id::Schedule(sid) = id {
            let schedule = st
                .schedules
                .get(&sid)
                .ok_or_else(|| anyhow::anyhow!("no job schedule {id}"))?;
            return Ok(port::Status {
                id: id.to_string(),
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
        Ok(port::Status {
            id: id.to_string(),
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

    fn result(&self, id: &str) -> Result<String> {
        let id = parse_id(id)?;
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

    fn cancel(&self, id: &str, notify: bool) -> Result<String> {
        let id = parse_id(id)?;
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
}

impl JobService {
    fn now(&self, spec: Spec, now: i64) -> Result<port::Receipt> {
        let mut st = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        if spec.needs_slot {
            anyhow::ensure!(
                st.running < self.inner.values.max_concurrent,
                "the job pool is full"
            );
        }
        let id = st.task_id();
        let output = if spec.output {
            Some(self.inner.dir.create(id)?)
        } else {
            None
        };
        let mut task = Job::new(id, spec, output, None);
        task.state = TaskState::Running;
        task.started = Some(now);
        if task.needs_slot {
            st.running += 1;
        }
        st.tasks.insert(id, task);
        drop(st);
        scheduler::run_job(self.inner.state.clone(), self.inner.bell.clone(), id);
        Ok(port::Receipt {
            id: id.to_string(),
            text: format!("{id} created"),
        })
    }

    fn after(&self, spec: Spec, delay: std::time::Duration, now: i64) -> Result<port::Receipt> {
        let mut st = self.inner.state.lock().unwrap_or_else(|e| e.into_inner());
        let id = st.task_id();
        let output = if spec.output {
            Some(self.inner.dir.create(id)?)
        } else {
            None
        };
        let at = now.saturating_add(delay.as_secs() as i64);
        let task = Job::new(id, spec, output, Some(at));
        st.tasks.insert(id, task);
        drop(st);
        self.inner.bell.1.notify_all();
        Ok(port::Receipt {
            id: id.to_string(),
            text: format!("{id} created for {}", stamp(at)),
        })
    }

    fn every(&self, spec: Spec, cron: Cron, count: Option<u32>, now: i64) -> Result<port::Receipt> {
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
        Ok(port::Receipt {
            id: Id::Schedule(sid).to_string(),
            text: format!("s{sid} created for {}", stamp(next)),
        })
    }

    fn fill(&self, mut spec: Spec) -> Spec {
        if spec.timeout.is_none() {
            spec.timeout = Some(std::time::Duration::from_secs(
                self.inner.values.timeout_secs,
            ));
        }
        spec
    }
}

/// One id as the model wrote it.
fn parse_id(text: &str) -> Result<Id> {
    Id::parse(text).ok_or_else(|| anyhow::anyhow!("`id` must look like `k1`, `s1`, or `s1.1`"))
}

fn stamp(unix: i64) -> String {
    let (y, m, d, h, min, s) = time::local_parts(unix);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{min:02}:{s:02}")
}
