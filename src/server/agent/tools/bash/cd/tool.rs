//! The `cd` tool: give the session a new footing.

use anyhow::Result;

use crate::server::agent::tools::contract::env::Env;
use crate::server::agent::tools::contract::output::ToolOutput;
use crate::server::agent::tools::contract::view::CallRecord;
use crate::server::agent::tools::spec::{Args, Kind, Param, Tool, ToolProgress};
use crate::server::agent::tools::summary;
use crate::server::agent::tools::workspace;

const PATH: usize = 0;

const PARAMS: &[Param] = &[Param::req(
    "path",
    Kind::Text,
    "Target directory. `./`, `../`, `~/`, and absolute paths allowed.",
)];

pub(crate) const TOOL: Tool = Tool {
    name: "cd",
    about: "Change the working directory for the rest of the session. Later tool calls and relative paths resolve from\n\
            the new one. Directories only, inside the workspace, the home directory or the temp directory but never\n\
            the home directory itself; confirm with `pwd` or `ls`.",
    params: PARAMS,
    run,
    summary,
};

/// Change footing. The target must exist, be a directory, and land inside the workspace, the
/// home directory or the temp dir; the home directory itself is never a workspace.
fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let path = args.str(PATH)?;
    let from = env.workspace.cwd().to_path_buf();
    let real = workspace::resolve_cwd(env.workspace.root(), &from, &path)?;
    env.workspace.chdir(real.clone());
    Ok(ToolOutput::with_memo(
        format!("working directory changed to {}", real.display()),
        serde_json::json!({
            "kind": "cwd",
            "path": real.to_string_lossy(),
            "from": from.to_string_lossy(),
        }),
    ))
}

/// One line for a history listing: where the session moved to.
fn summary(call: &CallRecord) -> String {
    summary::clip(&summary::field(call, "path").unwrap_or_default(), 96)
}
