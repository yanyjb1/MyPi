//! The `alias` tool: name shell commands for later calls.

use anyhow::{Result, anyhow, bail};
use serde_json::{Value, json};

use super::super::bash::{guard, segment};
use crate::server::agent::tools::contract::env::Env;
use crate::server::agent::tools::contract::output::{AliasOp, Effect, ToolOutput};
use crate::server::agent::tools::contract::view::CallRecord;
use crate::server::agent::tools::spec::{Args, Item, Kind, Param, Tool, ToolProgress};
use crate::server::agent::tools::summary;

const TYPE: usize = 0;
const ALIASES: usize = 1;
const NAMES: usize = 2;

const ALIAS_FIELDS: &[Param] = &[
    Param::new("name", Kind::Text, "Alias name."),
    Param::new(
        "command",
        Kind::Text,
        "The command the alias expands to; one line.",
    ),
];

const PARAMS: &[Param] = &[
    Param::new(
        "type",
        Kind::Choice(&["add", "del", "list"]),
        "`add` defines aliases, `list` shows them, `del` deletes them.",
    ),
    Param::new(
        "aliases",
        Kind::Objects(ALIAS_FIELDS),
        "type=add: one item per alias. One item's failure never stops the ones after it.",
    ),
    Param::new("names", Kind::TextList, "type=del: the alias names to delete."),
];

pub(crate) const TOOL: Tool = Tool {
    name: "alias",
    about: "Maintain shell aliases for this session.",
    fields: "type is always needed. type=add: aliases, one item per alias with name and command. \
            type=del: names. type=list: nothing else.",
    params: PARAMS,
    run,
    summary,
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    match args.choice(TYPE)?.as_str() {
        "add" => add(env, args),
        "list" => list(env, args),
        "del" => del(env, args),
        _ => unreachable!("Args::choice validates type"),
    }
}

fn add(env: &mut Env, args: &Args) -> Result<ToolOutput> {
    let items = args.objects(ALIASES)?;
    anyhow::ensure!(!items.is_empty(), "`aliases` must name at least one alias");
    anyhow::ensure!(
        args.str_list(NAMES)?.is_empty(),
        "`names` is only for type=del"
    );

    let mut lines = String::new();
    let mut effects = Vec::new();
    let mut failed = 0usize;
    for item in &items {
        match add_one(env, item) {
            Ok((line, effect)) => {
                lines.push_str(&line);
                lines.push('\n');
                effects.push(effect);
            }
            Err(e) => {
                if items.len() == 1 {
                    return Err(e);
                }
                failed += 1;
                lines.push_str(&format!("[{}] {e:#}\n", item.label()));
            }
        }
    }

    let mut produced = ToolOutput::text(lines.trim_end().to_string());
    for effect in effects {
        produced = produced.effect(effect);
    }
    if failed > 0 {
        produced = produced.failure(format!("{failed} of {} aliases not added", items.len()));
    }
    Ok(produced)
}

fn add_one(env: &mut Env, item: &Item) -> Result<(String, Effect)> {
    let name = alias_name(&item.text("name")?)?;
    anyhow::ensure!(
        !env.aliases.fixed(&name),
        "that name is fixed by the config; pick another name"
    );
    let command = item.text("command")?.trim().to_string();
    anyhow::ensure!(!command.is_empty(), "alias command must not be empty");
    anyhow::ensure!(
        !command.contains('\n') && !command.contains('\r'),
        "alias command must be one line"
    );

    let warning = vet(env, &command)?;
    env.aliases.set(name.clone(), command.clone());
    let mut line = format!("alias added: {name} = {command}");
    if let Some(w) = warning {
        line = format!("[warn: {w}]\n{line}");
    }
    let effect = Effect::Alias {
        op: AliasOp::Add,
        name,
        command: Some(command),
        ts: String::new(),
    };
    Ok((line, effect))
}

fn del(env: &mut Env, args: &Args) -> Result<ToolOutput> {
    let names = args.str_list(NAMES)?;
    anyhow::ensure!(!names.is_empty(), "`names` must name at least one alias");
    anyhow::ensure!(
        args.objects(ALIASES)?.is_empty(),
        "`aliases` is only for type=add"
    );

    let mut lines = String::new();
    let mut effects = Vec::new();
    let mut failed = 0usize;
    for raw in &names {
        match del_one(env, raw) {
            Ok((line, effect)) => {
                lines.push_str(&line);
                lines.push('\n');
                effects.push(effect);
            }
            Err(e) => {
                if names.len() == 1 {
                    return Err(e);
                }
                failed += 1;
                lines.push_str(&format!("[{raw}] {e:#}\n"));
            }
        }
    }

    let mut produced = ToolOutput::text(lines.trim_end().to_string());
    for effect in effects {
        produced = produced.effect(effect);
    }
    if failed > 0 {
        produced = produced.failure(format!("{failed} of {} aliases not deleted", names.len()));
    }
    Ok(produced)
}

fn del_one(env: &mut Env, raw: &str) -> Result<(String, Effect)> {
    let name = alias_name(raw)?;
    if env.aliases.unset(&name).is_none() {
        if env.aliases.fixed(&name) {
            bail!("that name is fixed by the config; not the session's to delete");
        }
        bail!("no alias by that name");
    }
    let effect = Effect::Alias {
        op: AliasOp::Delete,
        name: name.clone(),
        command: None,
        ts: String::new(),
    };
    Ok((format!("alias deleted: {name}"), effect))
}

fn list(env: &Env, args: &Args) -> Result<ToolOutput> {
    anyhow::ensure!(
        args.objects(ALIASES)?.is_empty() && args.str_list(NAMES)?.is_empty(),
        "type=list takes nothing else"
    );
    let rows: Vec<Value> = env
        .aliases
        .iter()
        .map(|(name, command, source)| {
            json!({ "name": name, "command": command, "source": source.tag() })
        })
        .collect();
    let memo = Value::Array(rows);
    let text = serde_json::to_string_pretty(&memo).unwrap_or_default();
    Ok(ToolOutput::with_memo(text, memo))
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

fn alias_name(raw: &str) -> Result<String> {
    let name = raw.trim().to_string();
    // The scanner is the only judge of a name: it must be one whole segment head.
    let usable = !name.contains('=') && segment::heads(&name) == [(0, name.len())];
    anyhow::ensure!(usable, "alias name must be a word that can start a command");
    Ok(name)
}

/// One line for a history listing: the op, plus the names it carries.
fn summary(call: &CallRecord) -> String {
    let op = summary::field(call, "type").unwrap_or_default();
    match op.as_str() {
        "add" => {
            let items = summary::array(call, "aliases");
            let first = items
                .first()
                .map(|item| {
                    let name = item.get("name").and_then(Value::as_str).unwrap_or_default();
                    let command = item
                        .get("command")
                        .and_then(Value::as_str)
                        .unwrap_or_default();
                    format!("{name} => {command}")
                })
                .unwrap_or_default();
            summary::clip(&summary::counted(&first, items.len()), 96)
        }
        "del" => {
            let names = summary::array(call, "names");
            let first = names
                .first()
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string();
            summary::clip(&summary::counted(&format!("del {first}"), names.len()), 96)
        }
        _ => summary::clip(&op, 96),
    }
}
