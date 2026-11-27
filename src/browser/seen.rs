//! What a session last read, so a second read of the same address answers only what changed.

use std::collections::HashMap;

use parking_lot::Mutex;

// A session's last read: the address it answered, and the prose it gave.
struct Record {
    address: String,
    text: String,
}

static SEEN: Mutex<Option<HashMap<i64, Record>>> = Mutex::new(None);

// The prose of a first read, or the header and the added lines of a re-read of the same address.
pub(crate) fn answer(session: i64, address: &str, text: &str) -> String {
    let mut guard = SEEN.lock();
    let records = guard.get_or_insert_with(HashMap::new);
    let out = match records.get(&session) {
        Some(before) if before.address == address => changed(&before.text, text),
        _ => text.to_string(),
    };
    records.insert(
        session,
        Record {
            address: address.to_string(),
            text: text.to_string(),
        },
    );
    out
}

// A session's record means nothing once the session ends.
pub(crate) fn forget(session: i64) {
    if let Some(records) = SEEN.lock().as_mut() {
        records.remove(&session);
    }
}

// The header and the lines the new prose adds, counted against what the old one still has spare.
fn changed(before: &str, after: &str) -> String {
    let mut left: HashMap<&str, usize> = HashMap::new();
    for line in before.lines().filter(|line| !line.trim().is_empty()) {
        *left.entry(line).or_default() += 1;
    }
    let mut added: Vec<&str> = Vec::new();
    for line in after.lines().filter(|line| !line.trim().is_empty()) {
        match left.get_mut(line) {
            Some(count) if *count > 0 => *count -= 1,
            _ => added.push(line),
        }
    }
    let gone: usize = left.values().sum();
    let mut out = header(gone, added.len());
    if !added.is_empty() {
        out.push('\n');
        for line in added {
            out.push_str(line);
            out.push('\n');
        }
    }
    out
}

// What the model is told before the added lines, and nothing else when the page did not change.
fn header(gone: usize, added: usize) -> String {
    match (gone, added) {
        (0, 0) => "Since your last read, nothing on this page changed.\n".to_string(),
        (0, added) => format!("Since your last read, {added} lines are new. New content below:\n"),
        (gone, 0) => {
            format!("Since your last read, this page has reduced {gone} lines. Nothing is new.\n")
        }
        (gone, added) => format!(
            "Since your last read, this page has reduced {gone} lines, and {added} lines are new. \
             New content below:\n"
        ),
    }
}
