// The Go layout: gofmt sets a text, and that answer is what a commit writes back.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use super::check::program;
use crate::platform::stop::Stop;
use crate::server::agent::tools::fs::gate;

// How long one layout round trip may take before its tree is killed.
const LIMIT: Duration = Duration::from_secs(10);

// The text as gofmt sets it. `None` = no gofmt, a stopped turn, or a text gofmt refused; a commit then leaves the file as it is.
pub(crate) fn layout(text: &str, _path: &Path, stop: &Arc<Stop>) -> Option<String> {
    let cmd = Command::new(program("gofmt")?);
    let ran = gate::run(cmd, Some(text.as_bytes()), Some(LIMIT), stop).ok()?;
    (ran.code == Some(0)).then_some(ran.out)
}
