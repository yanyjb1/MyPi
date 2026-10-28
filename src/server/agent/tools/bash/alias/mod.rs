//! alias: name a bash command for later calls.

use anyhow::{Result, anyhow};

use super::bash::guard;
use crate::server::agent::tools::spec::{Args, Env, Kind, Param, Tool, ToolOutput, ToolProgress};
use crate::server::storage::{AliasOp, EffectKind};

const OP: usize = 0;
const NAME: usize = 1;
const COMMAND: usize = 2;

pub(super) const PARAMS: &[Param] = &[
    Param::req(
        "op",
        Kind::Choice(&["add", "list", "remove"]),
        "add: define an alias; list: show current aliases; remove: delete one.",
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
    about: "Maintain bash aliases. An alias expands only the first token, only once, and only before the bash guard. It cannot contain variables, functions, or newlines. `add` and `remove` take effect for later bash calls in this round.",
    params: PARAMS,
    run,
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    match args.choice(OP)?.as_str() {
        "add" => add(env, args),
        "list" => Ok(ToolOutput::text(list(env))),
        "remove" => remove(env, args),
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

    let cwd = env.workspace.cwd().to_path_buf();
    let zone = cwd.canonicalize().unwrap_or(cwd);
    let warning = match guard::classify(&command, &zone) {
        guard::Verdict::Block(hits) => {
            let ids: Vec<&str> = hits.iter().map(|h| h.rule_id).collect();
            return Err(anyhow!("alias blocked: {}", ids.join("+")));
        }
        guard::Verdict::Warn(hits) => {
            Some(hits.iter().map(|h| h.rule_id).collect::<Vec<_>>().join("+"))
        }
        guard::Verdict::Allow => None,
    };

    env.aliases.insert(name.clone(), command.clone());
    let mut text = format!("alias added: {name} = {command}");
    if let Some(w) = warning {
        text = format!("[warn: {w}]\n{text}");
    }
    Ok(ToolOutput::text(text).effect(EffectKind::Alias {
        op: AliasOp::Add,
        name,
        command: Some(command),
    }))
}

fn remove(env: &mut Env, args: &Args) -> Result<ToolOutput> {
    let name = alias_name(args)?;
    anyhow::ensure!(
        env.aliases.remove(&name).is_some(),
        "no alias named `{name}`"
    );
    Ok(
        ToolOutput::text(format!("alias removed: {name}")).effect(EffectKind::Alias {
            op: AliasOp::Remove,
            name,
            command: None,
        }),
    )
}

fn list(env: &Env) -> String {
    if env.aliases.is_empty() {
        return "(no aliases)".to_string();
    }
    env.aliases
        .iter()
        .map(|(name, command)| format!("{name} = {command}"))
        .collect::<Vec<_>>()
        .join("\n")
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
