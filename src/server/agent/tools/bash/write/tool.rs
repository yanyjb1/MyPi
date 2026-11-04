//! The `write` tool: overwrite a whole file.

use anyhow::{Context as _, Result};

use crate::server::agent::tools::contract::env::Env;
use crate::server::agent::tools::contract::output::ToolOutput;
use crate::server::agent::tools::contract::view::CallRecord;
use crate::server::agent::tools::spec::{Args, Kind, Param, Tool, ToolProgress};
use crate::server::agent::tools::summary;

const PATH: usize = 0;
const CONTENT: usize = 1;

const PARAMS: &[Param] = &[
    Param::req("path", Kind::Text, "File path. `./` and `../` allowed."),
    Param::req(
        "content",
        Kind::Text,
        "The file's full content, written verbatim.",
    ),
];

pub(crate) const TOOL: Tool = Tool {
    name: "write",
    about: "Write the whole file, **overwriting** any existing one. For new files, or when every line changes; a single-location\n\
            change is `edit`. Only inside the workspace (the session's start directory) and the temp directory; a missing\n\
            parent directory is refused. NEVER create documentation or emojis unless asked.",
    params: PARAMS,
    run,
    summary,
};

/// Write the whole file, **overwriting** an existing one. The counterpart of `edit`: `edit` refuses
/// ambiguous matches, `write` refuses nothing. The caller brings the full content.
fn run(env: &mut Env, args: &Args, _progress: &mut dyn FnMut(ToolProgress)) -> Result<ToolOutput> {
    let path = args.str(PATH)?;
    let content = args.str(CONTENT)?;

    let p = env.workspace.resolve(&path, false)?;
    if let Some(parent) = p.parent() {
        anyhow::ensure!(
            parent.is_dir(),
            "parent directory does not exist: {} (create it with bash mkdir -p first)",
            parent.display()
        );
    }
    let existed = p.exists();
    env.workspace
        .with_write(&p, || {
            crate::server::agent::tools::workspace::write_atomic(&p, &content)
        })
        .context("failed to write file")?;
    let bytes = content.len();
    Ok(ToolOutput::text(format!(
        "{} {} ({bytes} bytes)",
        if existed { "overwrote" } else { "created" },
        p.display()
    )))
}

/// One line for a history listing: which file.
fn summary(call: &CallRecord) -> String {
    summary::clip(&summary::field(call, "path").unwrap_or_default(), 96)
}
