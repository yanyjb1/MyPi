//! The `alias` tool: name a shell command for later calls.

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use super::super::bash::guard;
use crate::server::agent::tools::contract::env::Env;
use crate::server::agent::tools::contract::output::{AliasOp, Effect, ToolOutput};
use crate::server::agent::tools::contract::view::CallRecord;
use crate::server::agent::tools::spec::{Args, Kind, Param, Tool, ToolProgress};
use crate::server::agent::tools::summary;

const OP: usize = 0;
const NAME: usize = 1;
const COMMAND: usize = 2;

const PARAMS: &[Param] = &[
    Param::req(
        "op",
        Kind::Choice(&["add", "list", "del"]),
        "add: define an alias; list: show current aliases; del: delete one.",
    ),
    Param::opt("name", Kind::Text, "Alias name."),
    Param::opt(
        "command",
        Kind::Text,
        "add: the command the alias expands to.",
    ),
];

pub(crate) const TOOL: Tool = Tool {
    name: "alias",
    about: "Maintain shell aliases. An alias expands only the first token, only once, and only before the guard. It cannot contain variables, functions, or newlines. `add` and `del` take effect for later bash calls in this round.",
    params: PARAMS,
    run,
    summary,
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    match args.choice(OP)?.as_str() {
        "add" => add(env, args),
        "list" => Ok(list(env)),
        "del" => del(env, args),
        other => Err(anyhow!("unknown alias op `{other}`")),
    }
}

fn add(env: &mut Env, args: &Args) -> Result<ToolOutput> {
    let name = alias_name(args)?;
    let command = args.str(COMMAND)?.trim().to_string();
    anyhow::ensure!(!command.is_empty(), "alias command must not be empty");
    anyhow::ensure!(
        !command.contains('\n') && !command.contains('\r'),
        "alias command must be one line"
    );

    let warning = vet(env, &command)?;
    env.aliases.insert(name.clone(), command.clone());
    let mut text = format!("alias added: {name} = {command}");
    if let Some(w) = warning {
        text = format!("[warn: {w}]\n{text}");
    }
    Ok(ToolOutput::text(text).effect(Effect::Alias {
        op: AliasOp::Add,
        name,
        command: Some(command),
        ts: String::new(),
    }))
}

fn del(env: &mut Env, args: &Args) -> Result<ToolOutput> {
    let name = alias_name(args)?;
    anyhow::ensure!(
        env.aliases.remove(&name).is_some(),
        "no alias named `{name}`"
    );
    Ok(
        ToolOutput::text(format!("alias deleted: {name}")).effect(Effect::Alias {
            op: AliasOp::Delete,
            name,
            command: None,
            ts: String::new(),
        }),
    )
}

fn list(env: &Env) -> ToolOutput {
    let rows: Vec<Value> = env
        .aliases
        .iter()
        .map(|(name, command)| json!({ "name": name, "command": command }))
        .collect();
    let memo = Value::Array(rows);
    let text = serde_json::to_string_pretty(&memo).unwrap_or_default();
    ToolOutput::with_memo(text, memo)
}

/// The classifier's verdict on a command that will later be run as written: a blocked command is
/// never stored, a warning is.
fn vet(env: &Env, command: &str) -> Result<Option<String>> {
    if !env.bash.shell.guarded() {
        return Ok(None);
    }
    let cwd = env.workspace.cwd().to_path_buf();
    let zone = cwd.canonicalize().unwrap_or(cwd);
    match guard::classify(command, &zone) {
        guard::Verdict::Allow => Ok(None),
        guard::Verdict::Warn(hits) => Ok(Some(
            hits.iter().map(|h| h.rule_id).collect::<Vec<_>>().join("+"),
        )),
        guard::Verdict::Block(hits) => {
            let ids: Vec<&str> = hits.iter().map(|h| h.rule_id).collect();
            Err(anyhow!("alias blocked: {}", ids.join("+")))
        }
    }
}

fn alias_name(args: &Args) -> Result<String> {
    let name = args.str(NAME)?.trim().to_string();
    let mut chars = name.chars();
    anyhow::ensure!(
        chars.next().is_some_and(|c| c.is_ascii_alphabetic()),
        "alias name must start with an ASCII letter"
    );
    anyhow::ensure!(
        chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'),
        "alias name allows only ASCII letters, digits, `_`, `-`"
    );
    Ok(name)
}

/// One line for a history listing: the op, plus the name and command it carries.
fn summary(call: &CallRecord) -> String {
    let op = summary::field(call, "op").unwrap_or_default();
    let name = summary::field(call, "name").unwrap_or_default();
    match (op.as_str(), summary::field(call, "command")) {
        ("add", Some(command)) => summary::clip(&format!("{name} => {command}"), 96),
        ("del", _) => summary::clip(&format!("del {name}"), 96),
        _ => summary::clip(&op, 96),
    }
}
