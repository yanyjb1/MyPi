// The TOML parser: the toml crate judges a whole document in process, and the byte it refused turns into the line it stands on.

use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};

use crate::server::agent::tools::fs::lang::verdict::{Outcome, Unparsed};
use crate::server::platform::stop::Stop;

// How many verdicts this process remembers.
const CACHE: usize = 32;

// Whether one text parses. A verdict is remembered: the text alone decides it, so nothing of the world around it can go stale.
pub(crate) fn format(text: &str, _path: &Path, _stop: &Arc<Stop>) -> Outcome {
    let key = hash(text);
    if let Some(hit) = remembered(&key) {
        return hit;
    }
    let outcome = judge(text);
    remember(key, &outcome);
    outcome
}

// One parse of a whole document: `toml::from_str` reads a document, where a `Value`'s own `parse` reads one bare value and refuses a document. The crate reports a byte span, and the line is counted up to its start.
fn judge(text: &str) -> Outcome {
    match toml::from_str::<toml::Table>(text) {
        Ok(_) => Outcome::Parsed,
        Err(e) => {
            let start = e.span().map_or(0, |span| span.start).min(text.len());
            Outcome::Unparsed(Unparsed {
                line: Some(text[..start].matches('\n').count() + 1),
                message: e.message().to_string(),
            })
        }
    }
}

// The verdicts this process has already reached, oldest first.
static MEMO: LazyLock<Mutex<Vec<(u64, Outcome)>>> = LazyLock::new(|| Mutex::new(Vec::new()));

fn remembered(key: &u64) -> Option<Outcome> {
    lock(&MEMO)
        .iter()
        .find(|(held, _)| held == key)
        .map(|(_, outcome)| outcome.clone())
}

fn remember(key: u64, outcome: &Outcome) {
    let mut held = lock(&MEMO);
    while held.len() >= CACHE {
        held.remove(0);
    }
    held.push((key, outcome.clone()));
}

// A lock that survives a panicking holder: the state behind it is a cache, never a promise.
fn lock<T>(held: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    held.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

// A text's content hash.
fn hash(text: &str) -> u64 {
    use std::hash::{Hash as _, Hasher as _};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}
