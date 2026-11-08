//! The horn: one event a sub-zone (or the main zone) puts out for the others.

use serde_json::Value;

/// One event on the bus. The name is the whole routing decision; the content is
/// whatever the shouter's counterpart expects, and nobody else looks at it.
#[derive(Debug, Clone, PartialEq)]
pub struct Shout {
    pub name: &'static str,
    pub content: Value,
}

impl Shout {
    pub fn new(name: &'static str, content: Value) -> Shout {
        Shout { name, content }
    }
}

/// The names this front end shouts today.
pub mod name {
    /// The main zone: the conversation's working directory. `{path, temporary}`.
    pub const CWD: &str = "cwd";
    /// History: the last request's context size. `{used}`.
    pub const USAGE: &str = "usage";
    /// A turn opened or closed. `{on}`.
    pub const BUSY: &str = "busy";
    /// A rewind put a message back: the editor takes it. `{text}`.
    pub const PREFILL: &str = "prefill";
    /// The pending queue changed size. `{count}`.
    pub const QUEUE: &str = "queue";
    /// A message the front end put out: history draws it now. `{text, prefix, suffix}`.
    pub const ECHO: &str = "echo";
    /// That message opened nothing: history takes the echo back. `{reason}`.
    pub const RETRACT: &str = "retract";
    /// History: one live increment arrived. No content; the spinner steps on it.
    pub const TICK: &str = "tick";
    /// The display language the panel reads. `{lang}`.
    pub const LANG: &str = "lang";
    /// The display currency the panel prices in. `{currency}`.
    pub const CURRENCY: &str = "currency";
    /// One model's config entry, addressed by its display. `{address, name, provider}`.
    pub const MODEL_ENTRY: &str = "model_entry";
}
