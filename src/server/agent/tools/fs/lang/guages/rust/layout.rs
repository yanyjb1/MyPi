// The Rust layout: rustfmt sets a text under the crate's own edition, and that answer is what a commit writes back.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use crate::server::agent::tools::fs::gate;
use crate::server::platform::stop::Stop;

// How long one layout round trip may take before its tree is killed: longer than a verdict, since a whole file comes back.
const LIMIT: Duration = Duration::from_secs(10);

// The text as rustfmt sets it. `None` = no rustfmt, a stopped turn, or a text rustfmt would not touch; a commit then leaves the file as it is.
pub(crate) fn layout(text: &str, path: &Path, stop: &Arc<Stop>) -> Option<String> {
    let edition = super::format::edition(path);
    let mut cmd = Command::new(gate::beside("rustfmt"));
    cmd.args(["--edition", &edition, "--emit", "stdout", "--quiet"]);
    let ran = gate::run(cmd, Some(text.as_bytes()), Some(LIMIT), stop).ok()?;
    (ran.code == Some(0)).then_some(ran.out)
}
