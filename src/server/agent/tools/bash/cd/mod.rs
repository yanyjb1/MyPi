//! cd: give the session a new footing.

use anyhow::{Context as _, Result};

use crate::server::agent::tools::spec::{Args, Env, Kind, Param, Tool, ToolOutput, ToolProgress};
use crate::server::agent::tools::workspace::canonicalize_lenient;

const PATH: usize = 0;

pub(super) const PARAMS: &[Param] = &[Param::req(
    "path",
    Kind::Text,
    "Target directory. `./`, `../`, `~/`, and absolute paths allowed.",
)];

pub(crate) const TOOL: Tool = Tool {
    name: "cd",
    about: "Change the working directory for the rest of the session. Later tool calls and relative paths resolve from\n\
            the new one. Directories only, and only within the workspace (the session's start directory); confirm\n\
            with pwd or ls.",
    params: PARAMS,
    run,
};

/// Change footing. It may stand in the workspace, `$HOME` or temp; never a system directory
/// (`/proc`, `/etc`, `/sys`), where every relative path would fall outside the zones.
fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let path = args.str(PATH)?;
    let p = path.trim();
    anyhow::ensure!(!p.is_empty(), "empty path");

    let target = if let Some(rest) = p.strip_prefix('~') {
        std::path::PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(rest)
    } else {
        std::path::PathBuf::from(p)
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

    let home = std::env::var_os("HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_default();
    let allowed = [
        canonicalize_lenient(env.workspace.root()),
        canonicalize_lenient(&home),
        canonicalize_lenient(&std::env::temp_dir()),
    ];
    anyhow::ensure!(
        allowed
            .iter()
            .any(|z| !z.as_os_str().is_empty() && real.starts_with(z)),
        "denied: {} is outside the workspace, $HOME, and /tmp",
        real.display()
    );

    env.workspace.chdir(real.clone());
    Ok(ToolOutput::text(format!(
        "working directory changed to {}",
        real.display()
    )))
}
