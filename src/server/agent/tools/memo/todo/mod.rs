//! todo: the model's list, kept **outside** its head. The state lives in the session, so a compaction
//! cannot take it; one bad entry fails the whole batch (rationale in `src/server/agent/README.md`).

mod ops;
pub(crate) mod state;

use anyhow::{Result, anyhow};

use crate::server::agent::tools::spec::{Args, Env, Kind, Param, Tool, ToolOutput, ToolProgress};

/// One phase's shape in `list`.
const PHASE_RECORD: &[Param] = &[
    Param::req("phase", Kind::Text, "Phase name."),
    Param::req("items", Kind::TextList, "Tasks in this phase."),
];

/// The order matches the index constants in `ops`.
pub(super) const PARAMS: &[Param] = &[
    Param::req(
        "op",
        Kind::Choice(ops::Op::ALL),
        "init: replace the whole list; append: add items; start/done/drop: move one item; block/unblock: park and release; rm: drop one item; view: read-only.",
    ),
    Param::opt(
        "list",
        Kind::Table(PHASE_RECORD),
        "init: phased list of [{phase, items}].",
    ),
    Param::opt("items", Kind::TextList, "init (flat) or append: task text."),
    Param::opt(
        "phase",
        Kind::Text,
        "append: which phase; defaults to the default phase.",
    ),
    Param::opt(
        "task",
        Kind::Text,
        "The item to act on, copied **verbatim**.",
    ),
    Param::opt("reason", Kind::Text, "block: why it is stuck."),
];

pub(crate) const TOOL: Tool = Tool {
    name: "todo",
    about: "Maintain a task checklist as an **external memo**: `init` a long task, then start/done each item as you go.\n\
            The state lives in the session, not in your head. `view` returns the **full** checklist; every other\n\
            operation answers in one line. `task` MUST match the item's text verbatim. Only one item is in_progress\n\
            (`start` demotes the previous one to pending). `block` requires `reason`. One bad entry fails the whole\n\
            batch; nothing is applied.",
    params: PARAMS,
    run,
};

fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let call = ops::Call::read(args)?;

    if call.op.is_read_only() {
        return Ok(ToolOutput::text(ops::rendered(&env.inbox.todo)));
    }

    // The list is the caller's own; a batch writes back in place, with no second copy.
    let (next, errors) = ops::apply(&env.inbox.todo, &call);
    if !errors.is_empty() {
        return Err(anyhow!(
            "{} not applied; the list is unchanged:\n- {}",
            call.op.as_str(),
            errors.join("\n- ")
        ));
    }
    let text = ops::confirm(&call, &next);
    env.inbox.todo = next;
    Ok(ToolOutput::text(text))
}
