//! memo: named notes. Reads both scopes; writes current only.

use anyhow::{Result, anyhow};

use crate::server::agent::tools::spec::{
    Args, Env, Kind, MemoReadOp, MemoView, Param, Tool, ToolOutput, ToolProgress,
};
use crate::server::storage::{EffectKind, MemoOp, MemoScope};

const OP: usize = 0;
const SCOPE: usize = 1;
const NAME: usize = 2;
const CONTENT: usize = 3;

pub(super) const PARAMS: &[Param] = &[
    Param::req(
        "op",
        Kind::Choice(&["add", "mod", "del", "show", "history", "list"]),
        "Operation. `show` reads the latest version; `history` reads every version.",
    ),
    Param::df(
        "scope",
        Kind::Choice(&["current", "global"]),
        "Where the memo lives. Writes are current only; reads accept both.",
        "current",
    ),
    Param::opt(
        "name",
        Kind::Text,
        "Memo name. Required by add, mod, del, show, and history.",
    ),
    Param::opt(
        "content",
        Kind::Text,
        "Full memo body. Required by add and mod.",
    ),
];

pub(crate) const TOOL: Tool = Tool {
    name: "memo",
    about: "Named notes shared with the user. Read current and global; write current only.\n\
            `show` reads the latest version, `history` reads every version, `list` lists names.\n\
            `add` creates, `mod` appends a version, `del` removes every version.",
    params: PARAMS,
    run,
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let op = args.choice(OP)?;
    let scope = match args.choice(SCOPE)?.as_str() {
        "current" => MemoScope::Current,
        "global" => MemoScope::Global,
        _ => unreachable!(),
    };
    match op.as_str() {
        "add" | "mod" | "del" => write(scope, &op, args),
        "show" | "history" | "list" => read(env, scope, &op, args),
        _ => Err(anyhow!("unknown memo operation `{op}`")),
    }
}

fn write(scope: MemoScope, op: &str, args: &Args) -> Result<ToolOutput> {
    if scope != MemoScope::Current {
        return Err(anyhow!("memo writes are current only"));
    }
    let name = name(args.str(NAME)?)?;
    let (memo_op, content) = match op {
        "add" => (MemoOp::Add, Some(args.str(CONTENT)?)),
        "mod" => (MemoOp::Mod, Some(args.str(CONTENT)?)),
        "del" => {
            if args.opt_any(CONTENT).is_some() {
                return Err(anyhow!("`del` does not take content"));
            }
            (MemoOp::Del, None)
        }
        _ => unreachable!(),
    };
    let text = format!("memo {op}: {name}");
    Ok(ToolOutput::text(text).effect(EffectKind::Memo {
        op: memo_op,
        scope: MemoScope::Current,
        anchor: None,
        name,
        content,
    }))
}

fn read(env: &Env, scope: MemoScope, op: &str, args: &Args) -> Result<ToolOutput> {
    let port = env
        .inbox
        .memo
        .as_deref()
        .ok_or_else(|| anyhow!("memo reads are unavailable"))?;
    let (read_op, name) = match op {
        "show" => (MemoReadOp::Show, Some(name(args.str(NAME)?)?)),
        "history" => (MemoReadOp::History, Some(name(args.str(NAME)?)?)),
        "list" => {
            if args.opt_any(NAME).is_some() || args.opt_any(CONTENT).is_some() {
                return Err(anyhow!("`list` takes only `scope`"));
            }
            (MemoReadOp::List, None)
        }
        _ => unreachable!(),
    };
    let out = port.read(env.inbox.session, scope, read_op, name.as_deref())?;
    Ok(ToolOutput::text(render(out)))
}

fn name(raw: String) -> Result<String> {
    let name = raw.trim();
    if name.is_empty() || name.chars().any(char::is_whitespace) {
        return Err(anyhow!("a memo name is non-empty and has no spaces"));
    }
    Ok(name.to_string())
}

fn render(memo: MemoView) -> String {
    match memo {
        MemoView::Names(names) if names.is_empty() => "(no memos)".to_string(),
        MemoView::Names(names) => names.join("\n"),
        MemoView::Content(content) => content,
        MemoView::History(versions) if versions.is_empty() => "(no history)".to_string(),
        MemoView::History(versions) => versions
            .into_iter()
            .map(|version| format!("#{} {}\n{}", version.id, version.ts, version.content))
            .collect::<Vec<_>>()
            .join("\n\n"),
    }
}
