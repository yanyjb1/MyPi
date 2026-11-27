// The web layout: biome sets a page, a stylesheet or a script as the repository itself configures it, and that answer is what a commit writes back.

use std::path::Path;
use std::sync::Arc;

use crate::platform::stop::Stop;

use super::format::round;

// The text as biome sets it. `None` = no biome, a stopped turn, or a formatter this repository leaves off; a commit then leaves the file as it is. No switch starts a formatter here: a commit writes what the repository has already decided its files read like.
pub(crate) fn layout(text: &str, path: &Path, stop: &Arc<Stop>) -> Option<String> {
    let ran = round(text, path, None, stop)?;
    if ran.stopped || ran.timed_out || ran.code != Some(0) {
        return None;
    }
    Some(ran.out)
}
