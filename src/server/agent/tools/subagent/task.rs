//! The `task` tool: one template, one child, one answer file.

use std::collections::BTreeMap;
use std::path::PathBuf;

use anyhow::{Result, bail};

use crate::server::agent::tools::contract::env::Env;
use crate::server::agent::tools::contract::output::{Effect, ToolOutput};
use crate::server::agent::tools::contract::view::CallRecord;
use crate::server::agent::tools::spec::{Args, Kind, Param, Tool, ToolProgress};
use crate::server::agent::tools::summary;

use super::answer;
use super::template;

const TEMPLATE: usize = 0;
const VARS: usize = 1;
const CWD: usize = 2;
const MAX: usize = 3;
const NAME: usize = 4;

pub(crate) const PARAMS: &[Param] = &[
    Param::new(
        "template",
        Kind::Text,
        "Template name, or a path when no named template exists.",
    ),
    Param::new(
        "vars",
        Kind::Object,
        "String values for the template's `{{...}}` slots.",
    ),
    Param::new(
        "cwd",
        Kind::Text,
        "Working directory; needs the parent's `cd` tool.",
    ),
    Param::new(
        "maxToolCalls",
        Kind::Number,
        "Tool-call budget for this child; the config value when omitted.",
    ),
    Param::new(
        "name",
        Kind::Text,
        "Answer file name; the template's stem when omitted.",
    ),
];

pub(crate) const TOOL: Tool = Tool {
    name: "task",
    about: "Spawn one subagent from a template and return at once. The child works in its own \
            session and writes an answer file; a wake reports when it ends. A child never gets \
            `task`: the tool cannot be handed down, so a template's tool list must not name it.",
    fields: "template is always needed. vars fills the template's slots. cwd sets the child's \
            directory and needs the parent's cd. maxToolCalls overrides the config budget. name \
            is the answer-file stem; a taken name gets a suffix.",
    params: PARAMS,
    run,
    summary,
};

/// What every child is told before its template body.
const CONTRACT: &str = "You are a subagent. You cannot ask the user anything, you cannot spawn \
                        another subagent, and only `final submit` ends your task.";

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let template_name = args.str(TEMPLATE)?;
    let vars = vars(args)?;
    let template = template::load(env, &template_name, &vars, &env.tools)?;
    let cwd = cwd(env, args)?;
    let max = match args.opt_int(MAX)? {
        Some(n) if n > 0 && n <= u32::MAX as i64 => Some(n as u32),
        Some(_) => bail!("`maxToolCalls` must be a positive integer"),
        None => None,
    }
    .unwrap_or(env.default_budget);
    let stem = args
        .opt_str(NAME)?
        .unwrap_or_else(|| template::stem(&template_name));
    let name = answer::free(&env.paths.answers, &stem)?;
    let path = answer::file(&env.paths.answers, &name);
    Ok(
        ToolOutput::text(format!("{name} created")).effect(Effect::Child {
            tools: template.tools,
            cwd,
            start: start_text(&template.body),
            max_tool_calls: max,
            answer: path,
        }),
    )
}

fn start_text(body: &str) -> String {
    format!(
        "{CONTRACT}\n\nBegin the task below. Use the tools you were given, put your answer in the \
         draft, then call `final submit`.\n\n{body}"
    )
}

fn cwd(env: &Env, args: &Args) -> Result<PathBuf> {
    let Some(raw) = args.opt_str(CWD)? else {
        return Ok(env.workspace.cwd());
    };
    anyhow::ensure!(env.allows("cd"), "`cwd` needs the parent's `cd` tool");
    let path = if std::path::Path::new(&raw).is_absolute() {
        PathBuf::from(&raw)
    } else {
        env.workspace.cwd().join(&raw)
    };
    let path = path
        .canonicalize()
        .map_err(|e| anyhow::anyhow!("cannot resolve cwd `{raw}`: {e}"))?;
    anyhow::ensure!(path.is_dir(), "cwd `{raw}` is not a directory");
    Ok(path)
}

fn vars(args: &Args) -> Result<BTreeMap<String, String>> {
    let Some(value) = args.opt_any(VARS) else {
        return Ok(BTreeMap::new());
    };
    let map = value
        .as_object()
        .ok_or_else(|| anyhow::anyhow!("`vars` must be an object of strings, not a string"))?;
    let mut out = BTreeMap::new();
    for (name, value) in map {
        let text = value
            .as_str()
            .ok_or_else(|| anyhow::anyhow!("`vars.{name}` must be a string"))?;
        out.insert(name.clone(), text.to_string());
    }
    Ok(out)
}

/// One line for a history listing: the template and the answer file's name.
fn summary(call: &CallRecord) -> String {
    let template = summary::field(call, "template").unwrap_or_default();
    let name = summary::field(call, "name").unwrap_or_else(|| template::stem(&template));
    summary::clip(&format!("{template} -> {name}"), 96)
}
