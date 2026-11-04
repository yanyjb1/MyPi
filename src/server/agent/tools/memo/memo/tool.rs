//! The `memo` tool: named notes. Reads both scopes; writes current only.

use anyhow::{Result, anyhow};
use serde_json::{Value, json};

use crate::server::agent::tools::contract::env::Env;
use crate::server::agent::tools::contract::output::{Effect, MemoOp, ToolOutput};
use crate::server::agent::tools::contract::view::CallRecord;
use crate::server::agent::tools::contract::view::{MemoReadOp, MemoScope, MemoView};
use crate::server::agent::tools::spec::{Args, Kind, Param, Tool, ToolProgress};
use crate::server::agent::tools::summary;

const OP: usize = 0;
const SCOPE: usize = 1;
const NAME: usize = 2;
const CONTENT: usize = 3;

const PARAMS: &[Param] = &[
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
    summary,
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let op = args.choice(OP)?;
    let scope = match args.choice(SCOPE)?.as_str() {
        "current" => MemoScope::Current,
        "global" => MemoScope::Global,
        _ => unreachable!(),
    };
    match op.as_str() {
        "add" | "mod" => write(scope, &op, args, true),
        "del" => write(scope, &op, args, false),
        "show" | "history" | "list" => read(env, scope, &op, args),
        _ => Err(anyhow!("unknown memo operation `{op}`")),
    }
}

/// A write never touches the store: it hands the loop one effect, landed with the result block.
fn write(scope: MemoScope, op: &str, args: &Args, has_content: bool) -> Result<ToolOutput> {
    if scope != MemoScope::Current {
        return Err(anyhow!("memo writes are current only"));
    }
    let name = name(args.str(NAME)?)?;
    let content = if has_content {
        Some(args.str(CONTENT)?)
    } else {
        if args.opt_any(CONTENT).is_some() {
            return Err(anyhow!("`del` does not take content"));
        }
        None
    };
    let memo_op = if content.is_some() {
        MemoOp::Set
    } else {
        MemoOp::Delete
    };
    Ok(
        ToolOutput::text(format!("memo {op}: {name}")).effect(Effect::Memo {
            op: memo_op,
            name,
            content,
            ts: String::new(),
        }),
    )
}

/// A read goes through the loop's port; the tool never opens the store.
fn read(env: &Env, scope: MemoScope, op: &str, args: &Args) -> Result<ToolOutput> {
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
    let view = env
        .source
        .memo(env.session)
        .read(env.session, scope, read_op, name.as_deref())?;
    let (text, memo) = render(view);
    Ok(ToolOutput::with_memo(text, memo))
}

fn name(raw: String) -> Result<String> {
    let name = raw.trim();
    if name.is_empty() || name.chars().any(char::is_whitespace) {
        return Err(anyhow!("a memo name is non-empty and has no spaces"));
    }
    Ok(name.to_string())
}

/// A read answers compact JSON; the text is that JSON, pretty-printed.
fn render(memo: MemoView) -> (String, Value) {
    let value = match memo {
        MemoView::Names(names) => json!(names),
        MemoView::Content(content) => json!({ "content": content }),
        MemoView::History(versions) => Value::Array(
            versions
                .into_iter()
                .map(|version| {
                    json!({ "id": version.id, "ts": version.ts, "content": version.content })
                })
                .collect(),
        ),
    };
    let text = serde_json::to_string_pretty(&value).unwrap_or_default();
    (text, value)
}

/// One line for a history listing: the operation only.
fn summary(call: &CallRecord) -> String {
    summary::clip(&summary::field(call, "op").unwrap_or_default(), 96)
}
