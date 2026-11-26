// The web layout: biome sets a page, a stylesheet or a script under the name it knows the file by, and that answer is what a commit writes back.

use std::path::Path;
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use super::check::{program, shown};
use super::kind;
use crate::server::agent::tools::fs::gate;
use crate::server::platform::stop::Stop;

// How long one layout round trip may take before its tree is killed.
const LIMIT: Duration = Duration::from_secs(10);

// The text as biome sets it. `None` = no biome, a stopped turn, or a text biome refused; a commit then leaves the file as it is.
pub(crate) fn layout(text: &str, path: &Path, stop: &Arc<Stop>) -> Option<String> {
    let mut cmd = Command::new(program()?);
    cmd.arg("format")
        .arg(format!("--stdin-file-path={}", shown(path)));
    // Biome leaves its HTML formatter off until asked, the same way the parser asks.
    if kind(path) == Some("html") {
        cmd.arg("--html-formatter-enabled=true");
    }
    let ran = gate::run(cmd, Some(text.as_bytes()), Some(LIMIT), stop).ok()?;
    (ran.code == Some(0)).then_some(ran.out)
}
