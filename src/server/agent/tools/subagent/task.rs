//! The `task` tool: resolve a template and hand one child to the job service.

use std::collections::BTreeMap;

use anyhow::{Result, bail};

use crate::server::agent::tools::spec::{Args, Env, Kind, Param, Tool, ToolOutput, ToolProgress};
use crate::server::storage::EffectKind;

const TEMPLATE: usize = 0;
const VARS: usize = 1;
const CWD: usize = 2;
const MAX: usize = 3;

pub(crate) const PARAMS: &[Param] = &[
    Param::req(
        "template",
        Kind::Text,
        "Template name, or a path when no named template exists.",
    ),
    Param::opt(
        "vars",
        Kind::Any,
        "Object of string values for the template's `{{...}}` slots.",
    ),
    Param::opt(
        "cwd",
        Kind::Text,
        "Working directory; needs the parent's `cd` tool.",
    ),
    Param::opt(
        "maxToolCalls",
        Kind::Number,
        "Tool-call budget for this child; defaults to config.",
    ),
];

pub(crate) const TOOL: Tool = Tool {
    name: "task",
    about: "Spawn one subagent from a template and return at once. The child works in its own \
            session and writes a final file; a wake reports when it ends.",
    params: PARAMS,
    run,
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let subagent = env
        .inbox
        .subagent
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("`task` is unavailable in this session"))?;
    anyhow::ensure!(
        subagent.is_parent(),
        "a child cannot spawn another subagent"
    );
    let template = args.str(TEMPLATE)?;
    let vars = vars(args)?;
    let cwd = args.opt_str(CWD)?;
    let max = match args.opt_int(MAX)? {
        Some(n) if n > 0 && n <= u32::MAX as i64 => Some(n as u32),
        Some(_) => bail!("`maxToolCalls` must be a positive integer"),
        None => None,
    };
    let receipt = subagent.task(env, &template, vars, cwd.as_deref(), max)?;
    Ok(
        ToolOutput::text(receipt.text()).effect(EffectKind::Subsession {
            session: receipt.child,
        }),
    )
}

fn vars(args: &Args) -> Result<BTreeMap<String, String>> {
    let Some(value) = args.opt_any(VARS) else {
        return Ok(BTreeMap::new());
    };
    let map = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("`vars` must be a JSON object"))?;
    let mut out = BTreeMap::new();
    for (name, value) in map {
        let text = value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("`vars.{name}` must be a string"))?;
        out.insert(name.clone(), text.to_string());
    }
    Ok(out)
}
