// The XML parser: quick-xml judges a text in process, and the byte it refused turns into the line it stands on. It reads tags and their pairing; an attribute's own quoting is not its business.

use std::path::Path;
use std::sync::{Arc, LazyLock, Mutex};

use quick_xml::Reader;
use quick_xml::events::Event;

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

// One parse, every event of it. quick-xml names a mismatched tag but reads an unclosed one as the stream's end, so the open tags are counted here: a document still holding a start tag at its end never closed it.
fn judge(text: &str) -> Outcome {
    let mut reader = Reader::from_str(text);
    let mut open: Vec<String> = Vec::new();
    loop {
        let at = reader.buffer_position() as usize;
        match reader.read_event() {
            Ok(Event::Eof) => {
                let Some(last) = open.last() else {
                    return Outcome::Parsed;
                };
                return Outcome::Unparsed(Unparsed {
                    line: None,
                    message: format!("unclosed element `<{last}>` at the document's end"),
                });
            }
            Ok(Event::Start(tag)) => {
                open.push(tag.name().as_ref().to_string());
            }
            Ok(Event::End(_)) => {
                open.pop();
            }
            Ok(_) => {}
            Err(e) => {
                let at = at.min(text.len());
                return Outcome::Unparsed(Unparsed {
                    line: Some(text[..at].matches('\n').count() + 1),
                    message: e.to_string(),
                });
            }
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
