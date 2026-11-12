//! The `alias` tool: name a shell command for later calls.

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use super::super::bash::{guard, segment};
use crate::server::agent::tools::contract::env::Env;
use crate::server::agent::tools::contract::output::{AliasOp, Effect, ToolOutput};
use crate::server::agent::tools::contract::view::CallRecord;
use crate::server::agent::tools::spec::{Args, Kind, Param, Tool, ToolProgress};
use crate::server::agent::tools::summary;

const TYPE: usize = 0;
const NAME: usize = 1;
const COMMAND: usize = 2;

const PARAMS: &[Param] = &[
    Param::new(
        "type",
        Kind::Choice(&["add", "list", "del"]),
        "Which operation this is.",
    ),
    Param::new("name", Kind::Text, "Alias name."),
    Param::new("command", Kind::Text, "The command the alias expands to."),
];

pub(crate) const TOOL: Tool = Tool {
    name: "alias",
    about: "Maintain shell aliases: `add` defines one, `list` shows them, `del` deletes one. The head \
            word of every command segment expands once, as text, before the guard. A name fixed by \
            the config is read-only for the session: `add` and `del` refuse it. `add` takes one \
            line. Changes take effect for later bash calls in this session.",
    fields: "type is always needed. type=add: name and command. type=del: name. \
            type=list: nothing else.",
    params: PARAMS,
    run,
    summary,
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    match args.choice(TYPE)?.as_str() {
        "add" => add(env, args),
        "list" => Ok(list(env)),
        "del" => del(env, args),
        other => Err(anyhow!("unknown alias type `{other}`")),
    }
}

fn add(env: &mut Env, args: &Args) -> Result<ToolOutput> {
    let name = alias_name(args)?;
    anyhow::ensure!(
        !env.aliases.fixed(&name),
        "`{name}` is fixed by the config; pick another name"
    );
    let command = args.str(COMMAND)?.trim().to_string();
    anyhow::ensure!(!command.is_empty(), "alias command must not be empty");
    anyhow::ensure!(
        !command.contains('\n') && !command.contains('\r'),
        "alias command must be one line"
    );

    let warning = vet(env, &command)?;
    env.aliases.set(name.clone(), command.clone());
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
    if env.aliases.unset(&name).is_none() {
        if env.aliases.fixed(&name) {
            bail!("`{name}` is fixed by the config; it is not the session's to delete");
        }
        bail!("no alias named `{name}`");
    }
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
        .map(|(name, command, source)| {
            json!({ "name": name, "command": command, "source": source.tag() })
        })
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
    let cwd = env.workspace.cwd();
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
    // The scanner is the only judge of a name: it must be one whole segment head.
    let usable = !name.contains('=') && segment::heads(&name) == [(0, name.len())];
    anyhow::ensure!(usable, "alias name must be a word that can start a command");
    Ok(name)
}

/// One line for a history listing: the op, plus the name and command it carries.
fn summary(call: &CallRecord) -> String {
    let op = summary::field(call, "type").unwrap_or_default();
    let name = summary::field(call, "name").unwrap_or_default();
    match (op.as_str(), summary::field(call, "command")) {
        ("add", Some(command)) => summary::clip(&format!("{name} => {command}"), 96),
        ("del", _) => summary::clip(&format!("del {name}"), 96),
        _ => summary::clip(&op, 96),
    }
}
