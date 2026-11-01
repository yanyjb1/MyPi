//! The `kairos` tool face: map one operation to a command or text job.

use std::sync::Arc;

use anyhow::{Result, bail};

use crate::server::agent::tools::contract::Env;
use crate::server::agent::tools::contract::ToolOutput;
use crate::server::agent::tools::contract::job::{
    JobHost, JobRunner, JobSpec, Schedule, Status,
};
use crate::server::agent::tools::spec::Args;

use super::command::{Command, CommandRunner};
use super::duration::parse as parse_duration;
use super::text::TextRunner;
use super::{CONTENT, COUNT, DELAY, EXPR, ID, NOTIFY, OP, TIMEOUT, TYPE};

pub(crate) fn run(env: &mut Env, args: &Args) -> Result<ToolOutput> {
    let jobs = env
        .source
        .jobs(env.session)
        .ok_or_else(|| anyhow::anyhow!("`kairos` is unavailable in this session"))?;
    let op = args.choice(OP)?;
    let text = match op.as_str() {
        "now" => {
            let kind = args.choice(TYPE)?;
            if kind != "command" {
                bail!("`text` runs on `after` or `every` only; `now` has nothing to wait for");
            }
            start(env, jobs.as_ref(), args, &kind, Schedule::Now)?
        }
        "after" => {
            let delay = parse_duration(&args.str(DELAY)?)?;
            let kind = args.choice(TYPE)?;
            start(env, jobs.as_ref(), args, &kind, Schedule::After(delay))?
        }
        "every" => {
            let expr = args.str(EXPR)?;
            let count = count(args)?;
            let kind = args.choice(TYPE)?;
            start(env, jobs.as_ref(), args, &kind, Schedule::Every { expr, count })?
        }
        "list" => list(jobs.as_ref()),
        "status" => render_status(&jobs.status(&args.str(ID)?)?),
        "result" => jobs.result(&args.str(ID)?)?,
        "cancel" => jobs.cancel(&args.str(ID)?, args.flag(NOTIFY)?)?,
        _ => unreachable!(),
    };
    Ok(ToolOutput::text(text))
}

fn start(
    env: &Env,
    jobs: &dyn JobHost,
    args: &Args,
    kind: &str,
    schedule: Schedule,
) -> Result<String> {
    let spec = match kind {
        "command" => {
            let content = args.str(CONTENT)?;
            let command = Command::prepare(env, &content)?;
            let runner: Arc<dyn JobRunner> = Arc::new(CommandRunner::new(
                command,
                env.bash.shell.clone(),
                env.export.clone(),
            ));
            JobSpec {
                runner,
                schedule,
                timeout: timeout(args)?,
                output: true,
                needs_slot: true,
                kind: "command",
                wake_on_done: true,
            }
        }
        "text" => JobSpec {
            runner: Arc::new(TextRunner {
                text: args.str(CONTENT)?,
            }),
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

fn timeout(args: &Args) -> Result<Option<std::time::Duration>> {
    args.opt_str(TIMEOUT)?
        .map(|text| parse_duration(&text))
        .transpose()
}

fn count(args: &Args) -> Result<Option<u32>> {
    match args.int(COUNT)? {
        -1 => Ok(None),
        n if n > 0 => Ok(Some(n as u32)),
        _ => bail!("`count` must be positive or -1"),
    }
}

fn list(jobs: &dyn JobHost) -> String {
    let rows = jobs.list();
    if rows.is_empty() {
        return "no jobs".to_string();
    }
    rows.into_iter()
        .map(|row| format!("{} {} {} {}B", row.id, row.kind, row.state, row.bytes))
        .collect::<Vec<_>>()
        .join("\n")
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
