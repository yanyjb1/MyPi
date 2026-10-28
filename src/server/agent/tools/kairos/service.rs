//! The `kairos` tool face: map one operation to a command or text job.

use std::sync::Arc;

use anyhow::{Result, bail};

use crate::server::agent::tools::Env;
use crate::server::agent::tools::jobs::{
    Cron, Id, JobCtx, JobOutcome, JobRunner, JobService, JobSpec, Schedule, Status, parse_duration,
};
use crate::server::agent::tools::spec::{Args, ToolOutput};

use super::command::{Command, CommandRunner};
use super::{CONTENT, COUNT, DELAY, EXPR, ID, NOTIFY, OP, TIMEOUT, TYPE};

pub(crate) fn run(env: &mut Env, args: &Args) -> Result<ToolOutput> {
    let jobs = env
        .inbox
        .jobs
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("`kairos` is unavailable in this session"))?;
    let op = args.choice(OP)?;
    let text = match op.as_str() {
        "now" => {
            let (content, timeout) = content_and_timeout(args)?;
            start(env, jobs, args, content, timeout, Schedule::Now)?
        }
        "after" => {
            let delay = parse_duration(&args.str(DELAY)?)?;
            let (content, timeout) = content_and_timeout(args)?;
            start(env, jobs, args, content, timeout, Schedule::After(delay))?
        }
        "every" => {
            let cron = Cron::parse(&args.str(EXPR)?)?;
            let count = count(args)?;
            let (content, timeout) = content_and_timeout(args)?;
            start(
                env,
                jobs,
                args,
                content,
                timeout,
                Schedule::Every(cron, count),
            )?
        }
        "list" => list(jobs),
        "status" => status(jobs, id(args)?)?,
        "result" => jobs.result(id(args)?)?,
        "cancel" => jobs.cancel(id(args)?, args.flag(NOTIFY)?)?,
        _ => unreachable!(),
    };
    Ok(ToolOutput::text(text))
}

fn start(
    env: &Env,
    jobs: &JobService,
    args: &Args,
    content: String,
    timeout: Option<std::time::Duration>,
    schedule: Schedule,
) -> Result<String> {
    let spec = match args.choice(TYPE)?.as_str() {
        "command" => {
            let command = Command::prepare(env, &content)?;
            let runner: Arc<dyn JobRunner> =
                Arc::new(CommandRunner::new(command, env.inbox.artifacts.clone()));
            JobSpec {
                runner,
                schedule,
                timeout,
                output: true,
                needs_slot: true,
                kind: "command",
                wake_on_done: true,
            }
        }
        "text" => JobSpec {
            runner: Arc::new(TextRunner { text: content }) as Arc<dyn JobRunner>,
            schedule,
            timeout: None,
            output: false,
            needs_slot: false,
            kind: "text",
            wake_on_done: true,
        },
        _ => unreachable!(),
    };
    Ok(jobs.start(spec)?.text)
}

struct TextRunner {
    text: String,
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

fn content_and_timeout(args: &Args) -> Result<(String, Option<std::time::Duration>)> {
    let content = args.str(CONTENT)?;
    let timeout = args
        .opt_str(TIMEOUT)?
        .map(|text| parse_duration(&text))
        .transpose()?;
    Ok((content, timeout))
}

fn count(args: &Args) -> Result<Option<u32>> {
    let count = args.opt_int(COUNT)?.unwrap_or(3);
    match count {
        -1 => Ok(None),
        n if n > 0 => Ok(Some(n as u32)),
        _ => bail!("`count` must be positive or -1"),
    }
}

fn id(args: &Args) -> Result<Id> {
    let text = args.str(ID)?;
    Id::parse(&text).ok_or_else(|| anyhow::anyhow!("`id` must look like `k1`, `s1`, or `s1.1`"))
}

fn list(jobs: &JobService) -> String {
    let rows = jobs.list();
    if rows.is_empty() {
        return "no jobs".to_string();
    }
    rows.into_iter()
        .map(|row| format!("{} {} {} {}B", row.id, row.kind, row.state, row.bytes))
        .collect::<Vec<_>>()
        .join("\n")
}

fn status(jobs: &JobService, id: Id) -> Result<String> {
    Ok(render_status(&jobs.status(id)?))
}

fn render_status(status: &Status) -> String {
    let mut text = format!("{} {}", status.id, status.state);
    if let Some(exit) = status.exit {
        text.push_str(&format!(", exit {exit}"));
    }
    if status.duration > 0 {
        text.push_str(&format!(", {}s", status.duration));
    }
    if status.bytes > 0 {
        text.push_str(&format!(", {} bytes", status.bytes));
    }
    if status.truncated {
        text.push_str(", truncated");
    }
    if let Some(next) = status.next {
        text.push_str(&format!(", next {}", stamp(next)));
    }
    if let Some(remaining) = status.remaining {
        text.push_str(&format!(", remaining {remaining}"));
    }
    if status.skipped > 0 {
        text.push_str(&format!(", skipped {}", status.skipped));
    }
    text
}

fn stamp(unix: i64) -> String {
    let (y, m, d, h, min, s) = crate::server::platform::time::local_parts(unix);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{min:02}:{s:02}")
}
