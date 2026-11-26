// The Python layout: ruff sets a text under the name it would have on disk, and that answer is what a commit writes back.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use super::check::program;
use crate::server::agent::tools::fs::gate;
use crate::server::platform::stop::Stop;

// How long one layout round trip may take before its tree is killed.
const LIMIT: Duration = Duration::from_secs(10);

// The text as ruff sets it. `None` = no ruff, a stopped turn, or a text ruff refused; a commit then leaves the file as it is.
pub(crate) fn layout(text: &str, path: &Path, stop: &Arc<Stop>) -> Option<String> {
    let mut cmd = Command::new(program("ruff")?);
    cmd.args(["format", "--stdin-filename"]).arg(path).arg("-");
    let ran = gate::run(cmd, Some(text.as_bytes()), Some(LIMIT), stop).ok()?;
    (ran.code == Some(0)).then_some(ran.out)
}
