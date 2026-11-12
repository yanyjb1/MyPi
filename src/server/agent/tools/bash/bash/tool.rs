//! The `bash` tool: raw text through the gate, then one shell line.

use anyhow::Result;

use super::{admit, shell};
use crate::server::agent::tools::contract::env::Env;
use crate::server::agent::tools::contract::output::ToolOutput;
use crate::server::agent::tools::contract::view::CallRecord;
use crate::server::agent::tools::spec::{Args, Kind, Param, Tool, ToolProgress};
use crate::server::agent::tools::summary;

const COMMAND: usize = 0;

const PARAMS: &[Param] = &[Param::new(
    "command",
    Kind::Text,
    "Shell commands; lines run in order.",
)];

pub(crate) const TOOL: Tool = Tool {
    name: "bash",
    about: "Run shell commands in the session's shell (`bash` or `zsh` on unix, `pwsh` on windows). \
            One call may carry several lines, run in order as one script. Pipes, redirection, and\n\
            composition are fine. The working directory is the permitted zone: deleting or moving things\n\
            inside it is allowed, but rm/mv/find -delete that reaches outside (including ~ and system\n\
            paths like /etc) is refused outright. Catastrophic patterns are blocked the same way: `dd`\n\
            to a block device, fork bombs, reverse shells, `curl | sh`.\n\
            A `#<ref>` token (e.g. `#3xl`) stands for a stored artifact — it is replaced by the\n\
            path of a file holding that whole text, so `grep x #3xl` and `cat #3xl` both work; a\n\
            token that names nothing stays literal text.",
    fields: "command is the only field.",
    params: PARAMS,
    run,
    summary,
};

fn run(env: &mut Env, args: &Args, progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let command = args.str(COMMAND)?;
    let raw = command.trim();
    anyhow::ensure!(!raw.is_empty(), "empty command");
    let admitted = admit::admit(env, raw)?;

    let cwd = env.workspace.cwd();
    let (text, exit) = shell::run(
        &env.bash.shell,
        &cwd,
        &admitted.cmd,
        env.bash.timeout,
        &env.stop,
        progress,
    )?;

    // The oversized tail is the front door's business (file it, append the ref); the tool hands up text.
    let text = match admitted.warn {
        Some(ids) => format!("[warn: {ids}]\n{text}"),
        None => text,
    };
    // A stop is the front door's to report; a deadline has no one else to speak for it.
    let out = ToolOutput::text(text);
    Ok(match exit.killed && !exit.canceled {
        true => out.failure(format!("timed out after {}s", exit.timeout_secs)),
        false => out,
    })
}

/// One line for a history listing: the command, flattened, and whether it failed.
fn summary(call: &CallRecord) -> String {
    let command = summary::field(call, "command").unwrap_or_else(|| call.args.clone());
    summary::clip(&command, 96)
}
