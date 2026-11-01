//! The `cd` tool: give the session a new footing.

use anyhow::{Context as _, Result};

use crate::server::agent::tools::contract::env::Env;
use crate::server::agent::tools::contract::output::ToolOutput;
use crate::server::agent::tools::contract::view::CallRecord;
use crate::server::agent::tools::spec::{Args, Kind, Param, Tool, ToolProgress};
use crate::server::agent::tools::summary;
use crate::server::agent::tools::workspace::canonicalize_lenient;
use crate::server::platform::env::{home, temp_dir};

const PATH: usize = 0;

const PARAMS: &[Param] = &[Param::req(
    "path",
    Kind::Text,
    "Target directory. `./`, `../`, `~/`, and absolute paths allowed.",
)];

pub(crate) const TOOL: Tool = Tool {
    name: "cd",
    about: "Change the working directory for the rest of the session. Later tool calls and relative paths resolve from\n\
            the new one. Directories only, and only inside the workspace, the home directory or the temp directory;\n\
            confirm with `pwd` or `ls`.",
    params: PARAMS,
    run,
    summary: Some(summary),
};

/// Change footing. It may stand in the workspace, home or temp; never a system directory
/// (`/proc`, `/etc`, `/sys`), where every relative path would fall outside the zones.
fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let path = args.str(PATH)?;
    let p = path.trim();
    anyhow::ensure!(!p.is_empty(), "empty path");

    let home = home();
    let target = match p.strip_prefix('~') {
        Some(rest) => home.clone().unwrap_or_default().join(rest),
        None => std::path::PathBuf::from(p),
    };
    let target = if target.is_absolute() {
        target
    } else {
        env.workspace.cwd().join(target)
    };
    let real = target
        .canonicalize()
        .with_context(|| format!("no such dir: {p}"))?;
    anyhow::ensure!(real.is_dir(), "not a directory: {p}");

    let mut allowed = vec![canonicalize_lenient(env.workspace.root())];
    if let Some(dir) = &home {
        allowed.push(canonicalize_lenient(dir));
    }
    allowed.push(canonicalize_lenient(&temp_dir()));
    anyhow::ensure!(
        allowed
            .iter()
            .any(|z| !z.as_os_str().is_empty() && real.starts_with(z)),
        "denied: {} is outside the workspace, the home directory, and the temp directory",
        real.display()
    );

    env.workspace.chdir(real.clone());
    Ok(ToolOutput::text(format!(
        "working directory changed to {}",
        real.display()
    )))
}

/// One line for a history listing: where the session moved to.
fn summary(call: &CallRecord) -> String {
    let path = summary::field(call, "path").unwrap_or_default();
    summary::clip(&format!("-> {path}"), 96)
}
