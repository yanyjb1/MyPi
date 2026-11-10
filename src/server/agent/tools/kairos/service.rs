//! The `kairos` tool face: map one operation to a command or text job.

use std::sync::Arc;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::server::agent::tools::contract::env::Env;
use crate::server::agent::tools::contract::job::{JobHost, JobRunner, JobSpec, Schedule, Status};
use crate::server::agent::tools::contract::output::ToolOutput;
use crate::server::agent::tools::spec::Args;

use super::command::{Command, CommandRunner};
use super::duration::parse as parse_duration;
use super::text::TextRunner;
use super::{CONTENT, COUNT, DELAY, EXPR, ID, KIND, TIMEOUT, TYPE};

pub(crate) fn run(env: &mut Env, args: &Args) -> Result<ToolOutput> {
    let jobs = env
        .source
        .jobs(env.session)
        .ok_or_else(|| anyhow::anyhow!("`kairos` is unavailable in this session"))?;
    let op = args.choice(TYPE)?;
    match op.as_str() {
        "now" => {
            let kind = args.choice(KIND)?;
            if kind != "command" {
                bail!("`text` runs on `after` or `every` only; `now` has nothing to wait for");
            }
            start(env, jobs.as_ref(), args, &kind, Schedule::Now)
        }
        "after" => {
            let delay = parse_duration(&args.str(DELAY)?)?;
            let kind = args.choice(KIND)?;
            start(env, jobs.as_ref(), args, &kind, Schedule::After(delay))
        }
        "every" => {
            let expr = args.str(EXPR)?;
            let count = count(args)?;
            let kind = args.choice(KIND)?;
            start(
                env,
                jobs.as_ref(),
                args,
                &kind,
                Schedule::Every { expr, count },
            )
        }
        "list" => Ok(list(jobs.as_ref())),
        "status" => Ok(status(&jobs.status(&args.str(ID)?)?)),
        "result" => Ok(ToolOutput::text(jobs.result(&args.str(ID)?)?)),
        "cancel" => Ok(ToolOutput::text(jobs.cancel(&args.str(ID)?)?)),
        _ => unreachable!(),
    }
}

fn start(
    env: &Env,
    jobs: &dyn JobHost,
    args: &Args,
    kind: &str,
    schedule: Schedule,
) -> Result<ToolOutput> {
    let spec = match kind {
        "command" => {
            let content = args.str(CONTENT)?;
            let command = Command::prepare(env, &content)?;
            let runner: Arc<dyn JobRunner> =
                Arc::new(CommandRunner::new(command, env.bash.shell.clone()));
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
    let receipt = jobs.start(spec)?;
    Ok(memo(serde_json::json!({
        "id": &receipt.id,
        "kind": kind,
        "at": receipt.at,
    })))
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

fn list(jobs: &dyn JobHost) -> ToolOutput {
    let rows: Vec<Value> = jobs
        .list()
        .into_iter()
        .map(|row| {
            json!({
                "id": &row.id,
                "kind": &row.kind,
                "state": &row.state,
                "bytes": row.bytes,
            })
        })
        .collect();
    memo(Value::Array(rows))
}

fn status(status: &Status) -> ToolOutput {
    memo(json!({
        "id": &status.id,
        "state": &status.state,
        "exit": status.exit,
        "duration": status.duration,
        "bytes": status.bytes,
        "truncated": status.truncated,
        "next": status.next.map(stamp),
        "remaining": status.remaining,
        "skipped": status.skipped,
    }))
}

fn memo(value: Value) -> ToolOutput {
    let text = serde_json::to_string_pretty(&value).unwrap_or_default();
    ToolOutput::with_memo(text, value)
}

fn stamp(unix: i64) -> String {
    let (y, m, d, h, min, s) = crate::server::platform::time::local_parts(unix);
    format!("{y:04}-{m:02}-{d:02} {h:02}:{min:02}:{s:02}")
}
