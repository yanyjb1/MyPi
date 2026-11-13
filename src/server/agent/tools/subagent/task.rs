//! The `task` tool: templates in, children out, one answer file each.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use anyhow::{Context as _, Result, bail};

use crate::server::agent::tools::contract::env::Env;
use crate::server::config::paths::project_dir;
use crate::server::agent::tools::contract::output::{Effect, ToolOutput};
use crate::server::agent::tools::contract::view::CallRecord;
use crate::server::agent::tools::spec::{Args, Item, Kind, Param, Tool, ToolProgress};
use crate::server::agent::tools::summary;

use super::answer;
use super::template;

const TASKS: usize = 0;

/// One child a call asks for.
const TASK_FIELDS: &[Param] = &[
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

const PARAMS: &[Param] = &[Param::new(
    "tasks",
    Kind::Objects(TASK_FIELDS),
    "One item per child, spawned in this order. One item's failure never stops the ones after it.",
)];

pub(crate) const TOOL: Tool = Tool {
    name: "task",
    about: "Spawn subagents from templates and return at once. Each child works in its own \
            session and writes an answer file; a wake reports when it ends. A child never gets \
            `task`: the tool cannot be handed down, so a template's tool list must not name it.",
    fields: "tasks is the only field a call needs: one item per child, with `template`, the \
            `vars` that fill its slots, the child's `cwd` (needs the parent's cd), its \
            `maxToolCalls` budget, and the `name` its answer file lands under.",
    params: PARAMS,
    run,
    summary,
};

/// What every child is told before its template body.
const CONTRACT: &str = "You are a subagent. You cannot ask the user anything, you cannot spawn \
                        another subagent, and only `final submit` ends your task.";

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let items = args.objects(TASKS)?;
    anyhow::ensure!(!items.is_empty(), "`tasks` must name at least one child");

    // The project folder is where this call is made, not where the server started.
    let project = project_dir(&env.workspace.cwd());
    let mut taken: BTreeSet<String> = BTreeSet::new();
    let mut text = String::new();
    let mut effects = Vec::new();
    let mut failed = 0usize;
    for item in &items {
        match spawn(env, &project, item, &mut taken) {
            Ok((name, effect)) => {
                text.push_str(&format!("{name} created\n"));
                effects.push(effect);
            }
            Err(e) => {
                if items.len() == 1 {
                    return Err(e);
                }
                failed += 1;
                text.push_str(&format!("[{}] {e:#}\n", item.label()));
            }
        }
    }

    let mut produced = ToolOutput::text(text.trim_end().to_string());
    for effect in effects {
        produced = produced.effect(effect);
    }
    if failed > 0 {
        produced = produced.failure(format!("{failed} of {} subagents not spawned", items.len()));
    }
    Ok(produced)
}

/// One child's effect: its tools, its start text, and the answer file it was given.
fn spawn(
    env: &Env,
    project: &std::path::Path,
    item: &Item,
    taken: &mut BTreeSet<String>,
) -> Result<(String, Effect)> {
    let template_name = item.text("template")?;
    let vars = vars(item)?;
    let template = template::load(env, project, &template_name, &vars, &env.tools)?;
    let cwd = cwd(env, item)?;
    let max = match item.opt_int("maxToolCalls")? {
        Some(n) if n > 0 && n <= u32::MAX as i64 => Some(n as u32),
        Some(_) => bail!("`{}.maxToolCalls` must be a positive integer", item.label()),
        None => None,
    }
    .unwrap_or(env.default_budget);
    let stem = item
        .opt_str("name")?
        .unwrap_or_else(|| template::stem(&template_name));
    let answers = project.join("answers");
    std::fs::create_dir_all(&answers)
        .with_context(|| format!("cannot create the answers folder {}", answers.display()))?;
    let name = answer::free_avoiding(&answers, &stem, taken)?;
    taken.insert(name.clone());
    let path = answer::file(&answers, &name);
    let effect = Effect::Child {
        tools: template.tools,
        cwd,
        start: start_text(&template.body),
        max_tool_calls: max,
        answer: path,
    };
    Ok((name, effect))
}

fn start_text(body: &str) -> String {
    format!(
        "{CONTRACT}\n\nBegin the task below. Use the tools you were given, put your answer in the \
         draft, then call `final submit`.\n\n{body}"
    )
}

fn cwd(env: &Env, item: &Item) -> Result<PathBuf> {
    let Some(raw) = item.opt_str("cwd")? else {
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

fn vars(item: &Item) -> Result<BTreeMap<String, String>> {
    let Some(map) = item.object("vars")? else {
        return Ok(BTreeMap::new());
    };
    let mut out = BTreeMap::new();
    for (name, value) in map {
        let text = value.as_str().ok_or_else(|| {
            anyhow::anyhow!("`{}.vars.{name}` must be a string", item.label())
        })?;
        out.insert(name, text.to_string());
    }
    Ok(out)
}

/// One line for a history listing: the first template and its answer file's name.
fn summary(call: &CallRecord) -> String {
    let items = summary::array(call, "tasks");
    let first = items
        .first()
        .map(|item| {
            let template = item
                .get("template")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let name = item
                .get("name")
                .and_then(serde_json::Value::as_str)
                .map(str::to_string)
                .unwrap_or_else(|| template::stem(template));
            format!("{template} -> {name}")
        })
        .unwrap_or_default();
    summary::clip(&summary::counted(&first, items.len()), 96)
}
