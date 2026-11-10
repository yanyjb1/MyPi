//! The built-in prompts.

/// A text file ends with a newline the original text did not have; drop it on the way out.
fn text(raw: &'static str) -> &'static str {
    raw.strip_suffix('\n').unwrap_or(raw)
}

/// The default system prompt.
pub fn system() -> &'static str {
    text(include_str!("SYSTEM.md"))
}

/// The compaction instruction.
pub fn compaction() -> &'static str {
    text(include_str!("COMPACTION.md"))
}
