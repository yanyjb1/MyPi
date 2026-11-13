//! The `kairos` tool face: map one operation to a batch of command or text jobs.

use std::sync::Arc;

use anyhow::{Result, bail};
use serde_json::{Value, json};

use crate::server::agent::tools::contract::env::Env;
use crate::server::agent::tools::contract::job::{JobHost, JobRunner, JobSpec, Schedule, Status};
use crate::server::agent::tools::contract::output::ToolOutput;
use crate::server::agent::tools::spec::{Args, Item};

use super::command::{Command, CommandRunner};
use super::duration::parse as parse_duration;
use super::text::TextRunner;
use super::tool::{ID, TASKS, TYPE};

pub(crate) fn run(env: &mut Env, args: &Args) -> Result<ToolOutput> {
    let jobs = env
        .source
        .jobs(env.session)
        .ok_or_else(|| anyhow::anyhow!("`kairos` is unavailable in this session"))?;
    let op = args.choice(TYPE)?;
    match op.as_str() {
        "now" | "after" | "every" => start(env, jobs.as_ref(), args, &op),
        _ => {
            // A read operation takes no tasks: the array belongs to the starting ones.
            anyhow::ensure!(
                args.objects(TASKS)?.is_empty(),
                "`tasks` is only for type=now/after/every"
            );
            match op.as_str() {
                "list" => Ok(list(jobs.as_ref())),
                "status" => Ok(status(&jobs.status(&args.str(ID)?)?)),
                "result" => Ok(ToolOutput::text(jobs.result(&args.str(ID)?)?)),
                "cancel" => Ok(ToolOutput::text(jobs.cancel(&args.str(ID)?)?)),
                _ => unreachable!("Args::choice validates type"),
            }
        }
    }
}

/// Start every task in the array, in order; one failure leaves the rest to start.
fn start(env: &Env, jobs: &dyn JobHost, args: &Args, op: &str) -> Result<ToolOutput> {
    let items = args.objects(TASKS)?;
    anyhow::ensure!(!items.is_empty(), "`tasks` must name at least one task");

    let mut text = String::new();
    let mut receipts: Vec<Value> = Vec::new();
    let mut failed = 0usize;
    for item in &items {
        match start_one(env, jobs, item, op) {
            Ok(value) => {
                text.push_str(&serde_json::to_string_pretty(&value)?);
                text.push('\n');
                receipts.push(value);
            }
            Err(e) => {
                if items.len() == 1 {
                    return Err(e);
                }
                failed += 1;
                // Keep one slot per item, so a card can still index receipts by position.
                receipts.push(Value::Null);
                text.push_str(&format!("[{}] {e:#}\n", item.label()));
            }
        }
    }

    let mut produced = ToolOutput::with_memo(text.trim_end().to_string(), Value::Array(receipts));
    if failed > 0 {
        produced = produced.failure(format!("{failed} of {} tasks not started", items.len()));
    }
    Ok(produced)
}

fn start_one(env: &Env, jobs: &dyn JobHost, item: &Item, op: &str) -> Result<Value> {
    let kind = item.choice("kind")?;
    if op == "now" && kind != "command" {
        bail!("`text` runs on `after` or `every` only; `now` has nothing to wait for");
    }
    let schedule = match op {
        "now" => Schedule::Now,
        "after" => Schedule::After(parse_duration(&item.text("delay")?)?),
        _ => Schedule::Every {
            expr: item.text("expr")?,
            count: count(item)?,
        },
    };

    let spec = match kind.as_str() {
        "command" => {
            let content = item.text("content")?;
            let command = Command::prepare(env, &content)?;
            let runner: Arc<dyn JobRunner> =
                Arc::new(CommandRunner::new(command, env.bash.shell.clone()));
            JobSpec {
                runner,
                schedule,
                timeout: timeout(item)?,
                output: true,
                needs_slot: true,
                kind: "command",
                wake_on_done: true,
            }
        }
        "text" => JobSpec {
            runner: Arc::new(TextRunner {
                text: item.text("content")?,
            }),
            schedule,
            timeout: None,
            output: false,
            needs_slot: false,
            kind: "text",
            wake_on_done: true,
        },
        _ => unreachable!("Item::choice validates kind"),
    };
    let receipt = jobs.start(spec)?;
    Ok(json!({
        "id": &receipt.id,
        "kind": kind,
        "at": receipt.at,
    }))
}

fn timeout(item: &Item) -> Result<Option<std::time::Duration>> {
    item.opt_str("timeout")?
        .map(|text| parse_duration(&text))
        .transpose()
}

fn count(item: &Item) -> Result<Option<u32>> {
    match item.int("count")? {
        -1 => Ok(None),
        n if n > 0 => Ok(Some(n as u32)),
        _ => bail!("`{}.count` must be positive or -1", item.label()),
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
