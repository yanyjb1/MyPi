// The R layout: air sets a text under the path it would have on disk, and that answer is what a commit writes back.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use super::format::program;
use crate::server::agent::tools::fs::gate;
use crate::server::platform::stop::Stop;

// How long one layout round trip may take before its tree is killed.
const LIMIT: Duration = Duration::from_secs(10);

// The text as air sets it. `None` = no air, a stopped turn, or a text air refused; a commit then leaves the file as it is.
pub(crate) fn layout(text: &str, path: &Path, stop: &Arc<Stop>) -> Option<String> {
    let mut cmd = Command::new(program("air")?);
    cmd.args(["format", "--stdin-file-path"]).arg(path);
    let ran = gate::run(cmd, Some(text.as_bytes()), Some(LIMIT), stop).ok()?;
    (ran.code == Some(0)).then_some(ran.out)
}
