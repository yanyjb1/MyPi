//! One scheduler thread per session: pools, clocks, skips and one wake per tick.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use anyhow::Result;

use crate::server::agent::tools::contract::job::{JobCtx, JobOutcome, OutputSink};
use crate::server::platform::time;

use super::Values;
use super::job::{Id, Job, State, TaskState};
use super::output::OutputDir;
use super::wake::{self, Wake, WakeSink};

/// The scheduler's wake: a flag inside the condvar's own mutex, so a ring lands either before the
/// wait or inside it, never between the two.
pub(crate) type Bell = Arc<(Mutex<bool>, Condvar)>;

/// Wake the scheduler: something it was waiting for has changed.
pub(crate) fn ring(bell: &Bell) {
    let mut rung = bell.0.lock().unwrap_or_else(|e| e.into_inner());
    *rung = true;
    bell.1.notify_all();
}

pub(crate) fn spawn(
    session: i64,
    values: Values,
    wake: Arc<dyn WakeSink>,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    bell: Bell,
    dir: OutputDir,
) -> std::io::Result<()> {
    std::thread::Builder::new()
        .name(format!("mypi-jobs-{session}"))
        .spawn(move || run(session, values, wake, state, stop, bell, dir))
        .map(|_| ())
}

/// What the thread does after one pass.
enum Next {
    /// Something is scheduled: wake at this point.
    At(Duration),
    /// Nothing is due, but a running job or a schedule may still need this thread.
    Idle,
    /// Nothing left: the thread ends.
    Done,
}

fn run(
    session: i64,
    values: Values,
    wake: Arc<dyn WakeSink>,
    state: Arc<Mutex<State>>,
    stop: Arc<AtomicBool>,
    bell: Bell,
    dir: OutputDir,
) {
    while !stop.load(Ordering::Relaxed) {
        match tick(session, values, &wake, &state, &bell, &dir) {
            Next::Done => return,
            Next::Idle => park(&bell, None),
            Next::At(wait) => park(&bell, Some(wait)),
        }
    }
}

/// Block on the bell: until the deadline, or until a job finishes or a schedule appears. A ring
/// that arrived while this pass ran returns at once.
fn park(bell: &Bell, wait: Option<Duration>) {
    let mut rung = bell.0.lock().unwrap_or_else(|e| e.into_inner());
    if *rung {
        *rung = false;
        return;
    }
    match wait {
        Some(duration) => {
            let (mut rung, _) = bell
                .1
                .wait_timeout(rung, duration)
                .unwrap_or_else(|e| e.into_inner());
            *rung = false;
        }
        None => {
            let mut rung = bell.1.wait(rung).unwrap_or_else(|e| e.into_inner());
            *rung = false;
        }
    }
}

fn tick(
    session: i64,
    values: Values,
    wake: &Arc<dyn WakeSink>,
    state: &Arc<Mutex<State>>,
    bell: &Bell,
    dir: &OutputDir,
) -> Next {
    let now = time::unix_now();
    let mut starts = Vec::new();
    let mut messages = Vec::new();
    let (wakes, next, done) = {
        let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
        start_after(&mut st, values, now, &mut starts);
        start_every(&mut st, values, now, &mut starts, &mut messages, dir);
        prune(&mut st, dir);
        st.pending_wakes.append(&mut messages);
        let wakes = std::mem::take(&mut st.pending_wakes);
        let next = deadline(&st);
        let done = !pending(&st);
        if done {
            st.alive = false;
        }
        (wakes, next, done)
    };
    if !wakes.is_empty() {
        wake.wake(Wake {
            session,
            text: wakes.join("\n"),
        });
    }
    let mut lost = false;
    for id in starts {
        if run_job(state.clone(), bell.clone(), id).is_err() {
            lost = true;
        }
    }
    // A job whose thread never started still owes a wake; one more tick delivers it.
    if lost {
        return Next::At(Duration::ZERO);
    }
    if done {
        return Next::Done;
    }
    match next {
        Some(at) => Next::At(Duration::from_secs((at - now).max(0) as u64)),
        None => Next::Idle,
    }
}

/// The earliest point at which a scheduled task or a cron entry becomes due.
fn deadline(st: &State) -> Option<i64> {
    let tasks = st
        .tasks
        .values()
        .filter(|task| matches!(task.state, TaskState::Scheduled | TaskState::Waiting))
        .filter_map(|task| task.start_at);
    let schedules = st.schedules.values().filter_map(|entry| entry.next);
    tasks.chain(schedules).min()
}

/// Whether anything could still need this thread: a schedule, or a task the pool has not finished.
fn pending(st: &State) -> bool {
    !st.schedules.is_empty()
        || st.tasks.values().any(|task| {
            matches!(
                task.state,
                TaskState::Scheduled | TaskState::Waiting | TaskState::Running
            )
        })
}

fn start_after(st: &mut State, values: Values, now: i64, starts: &mut Vec<Id>) {
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
        if task.needs_slot && st.running >= values.max_concurrent {
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
    values: Values,
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
            let full = schedule.spec.needs_slot && st.running >= values.max_concurrent;
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
                "{} skipped {} times before running",
                Id::Schedule(sid),
                schedule.skipped
            ));
            schedule.skipped = 0;
        }
        let spec = schedule.spec.clone();
        let output = if spec.output {
            match dir.create(run) {
                Ok(output) => Some(output),
                Err(e) => {
                    messages.push(format!("{run} failed to start: {e:#}"));
                    schedule.running = None;
                    continue;
                }
            }
        } else {
            None
        };
        let mut task = Job::new(run, spec, output, None);
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

pub(crate) fn run_job(state: Arc<Mutex<State>>, bell: Bell, id: Id) -> Result<()> {
    let worker_state = state.clone();
    let worker_bell = bell.clone();
    std::thread::Builder::new()
        .name(format!("mypi-jobs-run-{id}"))
        .spawn(move || {
            let (runner, timeout, cancel, needs_slot, mut output) = {
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
                    task.timeout,
                    task.cancel.clone(),
                    task.needs_slot,
                    output,
                )
            };
            let result = {
                let mut ctx = JobCtx {
                    cancel: cancel.clone(),
                    timeout,
                    output: output.as_mut().map(|file| file as &mut dyn OutputSink),
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
            let canceled = task.cancel.is_stopped() || task.state == TaskState::Canceled;
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
            ring(&worker_bell);
        })
        .map(|_| ())
        .map_err(|e| {
            settle_unstarted(&state, id, &e);
            ring(&bell);
            anyhow::Error::new(e)
        })
}

/// No thread: the job fails here, its pool slot returns, and its wake rides the next tick.
fn settle_unstarted(state: &Mutex<State>, id: Id, why: &std::io::Error) {
    let mut st = state.lock().unwrap_or_else(|e| e.into_inner());
    let mut needs_slot = false;
    let mut schedule = None;
    let mut notice = None;
    if let Some(task) = st.tasks.get_mut(&id) {
        task.state = TaskState::Failed;
        task.ended = Some(time::unix_now());
        task.text = Some(format!("the job's thread did not start: {why}"));
        let duration = task.duration();
        needs_slot = task.needs_slot;
        schedule = task.schedule;
        if task.wake_on_done {
            notice = Some(wake::job_status(
                id,
                TaskState::Failed,
                None,
                duration,
                task.bytes,
                task.truncated,
            ));
        }
    }
    if needs_slot {
        st.running = st.running.saturating_sub(1);
    }
    if let Some(sid) = schedule
        && let Some(entry) = st.schedules.get_mut(&sid)
        && entry.running == Some(id)
    {
        entry.running = None;
    }
    if let Some(text) = notice {
        st.pending_wakes.push(text);
    }
}
