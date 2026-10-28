//! One scheduler thread per session: pools, clocks, skips and one wake per tick.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use crate::server::config::KairosConfig;
use crate::server::platform::time;

use super::job::{Id, Job, State, TaskState};
use super::output::OutputDir;
use super::runner::{JobCtx, JobOutcome};
use super::wake::{self, Wake, WakeSink};

pub(crate) fn spawn(
    session: i64,
    cfg: KairosConfig,
    wake: Arc<dyn WakeSink>,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    bell: Arc<(Mutex<()>, Condvar)>,
    dir: OutputDir,
) {
    let _ = std::thread::Builder::new()
        .name(format!("mypi-jobs-{session}"))
        .spawn(move || run(session, cfg, wake, state, stop, bell, dir));
}

fn run(
    session: i64,
    cfg: KairosConfig,
    wake: Arc<dyn WakeSink>,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    bell: Arc<(Mutex<()>, Condvar)>,
    dir: OutputDir,
) {
    while !stop.load(Ordering::Relaxed) {
        tick(session, cfg, &wake, &state, &bell, &dir);
        let guard = bell.0.lock().unwrap_or_else(|e| e.into_inner());
        let _ = bell
            .1
            .wait_timeout(guard, Duration::from_millis(1000))
            .unwrap_or_else(|e| e.into_inner());
    }
}

fn tick(
    session: i64,
    cfg: KairosConfig,
    wake: &Arc<dyn WakeSink>,
    state: &Arc<Mutex<State>>,
    bell: &Arc<(Mutex<()>, Condvar)>,
    dir: &OutputDir,
) {
    let now = time::unix_now();
    let mut starts = Vec::new();
    let mut messages = Vec::new();
    let pending = {
        let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
        start_after(&mut st, cfg, now, &mut starts);
        start_every(&mut st, cfg, now, &mut starts, &mut messages, dir);
        prune(&mut st, dir);
        st.pending_wakes.append(&mut messages);
        std::mem::take(&mut st.pending_wakes)
    };
    if !pending.is_empty() {
        wake.wake(Wake {
            session,
            text: pending.join("\n"),
        });
    }
    for id in starts {
        run_job(session, state.clone(), bell.clone(), id);
    }
}

fn start_after(st: &mut State, cfg: KairosConfig, now: i64, starts: &mut Vec<Id>) {
    let ids: Vec<Id> = st
        .tasks
        .iter()
        .filter(|(_, t)| {
            matches!(t.state, TaskState::Scheduled | TaskState::Waiting)
                && t.start_at.is_some_and(|at| at <= now)
        })
        .map(|(id, _)| *id)
        .collect();
    for id in ids {
        let Some(task) = st.tasks.get_mut(&id) else {
            continue;
        };
        if task.needs_slot && st.running >= cfg.max_concurrent {
            task.state = TaskState::Waiting;
            continue;
        }
        task.state = TaskState::Running;
        task.started = Some(now);
        if task.needs_slot {
            st.running += 1;
        }
        starts.push(id);
    }
}

fn start_every(
    st: &mut State,
    cfg: KairosConfig,
    now: i64,
    starts: &mut Vec<Id>,
    messages: &mut Vec<String>,
    dir: &OutputDir,
) {
    let ids: Vec<u64> = st
        .schedules
        .iter()
        .filter(|(_, s)| s.next.is_some_and(|at| at <= now))
        .map(|(id, _)| *id)
        .collect();
    for sid in ids {
        let skip = {
            let Some(schedule) = st.schedules.get_mut(&sid) else {
                continue;
            };
            let running = schedule
                .running
                .and_then(|id| st.tasks.get(&id))
                .is_some_and(|t| t.state == TaskState::Running);
            let full = schedule.spec.needs_slot && st.running >= cfg.max_concurrent;
            if running || full {
                let first = schedule.skipped == 0;
                let reason = if running {
                    "still running"
                } else {
                    "pool full"
                };
                schedule.skipped = schedule.skipped.saturating_add(1);
                schedule.next = schedule.cron.next(now);
                Some((first, reason, schedule.skipped))
            } else {
                None
            }
        };
        if let Some((first, reason, count)) = skip {
            if first {
                messages.push(wake::skipped(Id::Schedule(sid), reason, count));
            }
            continue;
        }
        let Some(schedule) = st.schedules.get_mut(&sid) else {
            continue;
        };
        schedule.next = schedule.cron.next(now);
        schedule.next_run = schedule.next_run.saturating_add(1);
        let run = Id::Run(sid, schedule.next_run);
        schedule.runs.push_back(run);
        while schedule.runs.len() > dir.max_retained() {
            schedule.runs.pop_front();
        }
        if let Some(left) = schedule.remaining.as_mut() {
            *left = left.saturating_sub(1);
        }
        let remove = schedule.remaining == Some(0);
        if schedule.skipped > 0 {
            messages.push(format!(
                "job s{sid} skipped {} times before running",
                schedule.skipped
            ));
            schedule.skipped = 0;
        }
        let spec = schedule.spec.clone();
        let output = if spec.output {
            match dir.create(run) {
                Ok(output) => Some(output),
                Err(e) => {
                    messages.push(format!("job {run} failed to start: {e:#}"));
                    schedule.running = None;
                    continue;
                }
            }
        } else {
            None
        };
        let mut task = Job::new(run, spec, dir.task_dir(run), output, None);
        task.schedule = Some(sid);
        task.state = TaskState::Running;
        task.started = Some(now);
        if task.needs_slot {
            st.running += 1;
        }
        st.tasks.insert(run, task);
        schedule.running = Some(run);
        starts.push(run);
        if remove {
            st.schedules.remove(&sid);
        }
    }
}

fn prune(st: &mut State, dir: &OutputDir) {
    let max = dir.max_retained();
    let mut finished: Vec<(i64, Id)> = st
        .tasks
        .iter()
        .filter(|(_, t)| {
            matches!(
                t.state,
                TaskState::Done | TaskState::Failed | TaskState::Canceled
            ) && !t.consumed
                && t.output.is_some()
        })
        .map(|(id, t)| (t.ended.unwrap_or(i64::MAX), *id))
        .collect();
    finished.sort_by_key(|(end, _)| *end);
    let excess = finished.len().saturating_sub(max);
    for (_, id) in finished.into_iter().take(excess) {
        if let Some(task) = st.tasks.get_mut(&id) {
            task.state = TaskState::Expired;
            if let Some(output) = task.output.take() {
                output.discard();
            }
        }
        dir.remove_task_dir(id);
    }
    for schedule in st.schedules.values_mut() {
        while schedule.runs.len() > max {
            schedule.runs.pop_front();
        }
    }
    let max_tasks = max.saturating_mul(3).max(8);
    let mut stale: Vec<(i64, Id)> = st
        .tasks
        .iter()
        .filter(|(_, t)| {
            matches!(
                t.state,
                TaskState::Done | TaskState::Failed | TaskState::Canceled | TaskState::Expired
            )
        })
        .map(|(id, t)| (t.ended.unwrap_or(i64::MAX), *id))
        .collect();
    stale.sort_by_key(|(end, _)| *end);
    while st.tasks.len() > max_tasks {
        let Some((_, id)) = stale.first().copied() else {
            break;
        };
        stale.remove(0);
        st.tasks.remove(&id);
    }
}

pub(crate) fn run_job(
    session: i64,
    state: Arc<Mutex<State>>,
    bell: Arc<(Mutex<()>, Condvar)>,
    id: Id,
) {
    let worker_state = state.clone();
    let worker_bell = bell.clone();
    let _ = std::thread::Builder::new()
        .name(format!("mypi-jobs-run-{id}"))
        .spawn(move || {
            let (runner, dir, timeout, cancel, needs_slot, mut output) = {
                let mut st = worker_state.lock().unwrap_or_else(|e| e.into_inner());
                let Some(task) = st.tasks.get_mut(&id) else {
                    return;
                };
                let Some(runner) = task.runner.take() else {
                    return;
                };
                let output = task.output.take();
                (
                    runner,
                    task.dir.clone(),
                    task.timeout,
                    task.cancel.clone(),
                    task.needs_slot,
                    output,
                )
            };
            let result = {
                let mut ctx = JobCtx {
                    session,
                    job: id,
                    cancel: cancel.clone(),
                    dir: dir.clone(),
                    timeout,
                    output: output.as_mut(),
                };
                runner.run(&mut ctx)
            };
            let now = time::unix_now();
            let mut st = worker_state.lock().unwrap_or_else(|e| e.into_inner());
            let Some(task) = st.tasks.get_mut(&id) else {
                if needs_slot {
                    st.running = st.running.saturating_sub(1);
                }
                return;
            };
            let outcome = match result {
                Ok(outcome) => outcome,
                Err(e) => {
                    task.text = Some(format!("{e:#}"));
                    JobOutcome::failed()
                }
            };
            task.ended = Some(now);
            if let Some(output) = output {
                task.bytes = output.bytes();
                task.truncated = output.truncated();
                task.output = Some(output);
            }
            task.exit = outcome.exit;
            task.text = outcome.text.or(task.text.take());
            let canceled = task.cancel.load(Ordering::Relaxed) || task.state == TaskState::Canceled;
            task.state = if canceled {
                TaskState::Canceled
            } else if outcome.ok {
                TaskState::Done
            } else {
                TaskState::Failed
            };
            let needs_slot = task.needs_slot;
            let schedule = task.schedule;
            let wake_on_done = task.wake_on_done;
            let state = task.state;
            let exit = task.exit;
            let duration = task.duration();
            let bytes = task.bytes;
            let truncated = task.truncated;
            drop(task);
            if needs_slot {
                st.running = st.running.saturating_sub(1);
            }
            if let Some(sid) = schedule
                && let Some(schedule) = st.schedules.get_mut(&sid)
                && schedule.running == Some(id)
            {
                schedule.running = None;
            }
            if wake_on_done {
                let text = outcome.wake.unwrap_or_else(|| {
                    wake::job_status(id, state, exit, duration, bytes, truncated)
                });
                st.pending_wakes.push(text);
            }
            drop(st);
            worker_bell.1.notify_all();
        });
}
