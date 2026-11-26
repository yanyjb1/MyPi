// The CP layout: clang-format sets a text, but only where the project states its own style. Without a `.clang-format` above the file, clang-format falls back to LLVM's, and a commit must not restyle a project that never asked for that.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use super::check::program;
use crate::server::agent::tools::fs::gate;
use crate::server::platform::stop::Stop;

// How long one layout round trip may take before its tree is killed.
const LIMIT: Duration = Duration::from_secs(20);

// The style file clang-format looks for above a source, nearest first.
const STYLE: &str = ".clang-format";

// The text as clang-format sets it under the project's own style. `None` = no clang-format, no style file of the project's, a stopped turn, or a text clang-format refused.
pub(crate) fn layout(text: &str, path: &Path, stop: &Arc<Stop>) -> Option<String> {
    // The style file's directory, and the source's own name: clang-format reads the language from the name and the style from the file.
    let style = style_of(path)?;
    let named = style.parent()?.join(path.file_name()?);
    let mut cmd = Command::new(program("clang-format")?);
    cmd.arg("--style=file")
        .arg(format!("--assume-filename={}", named.display()))
        .arg("-");
    let ran = gate::run(cmd, Some(text.as_bytes()), Some(LIMIT), stop).ok()?;
    (ran.code == Some(0)).then_some(ran.out)
}

// The project's own style file at or above the source.
fn style_of(path: &Path) -> Option<PathBuf> {
    path.parent()?
        .ancestors()
        .map(|at| at.join(STYLE))
        .find(|file| file.is_file())
}
