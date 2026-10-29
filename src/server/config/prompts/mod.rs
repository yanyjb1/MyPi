//! The built-in prompts.

/// A text file ends with a newline the original text did not have; drop it on the way out.
fn text(raw: &'static str) -> &'static str {
    raw.strip_suffix('\n').unwrap_or(raw)
}

/// The default system prompt, also used by the built-in `oh-my-pi` profile.
pub fn omp_system() -> &'static str {
    text(include_str!("omp_system.md"))
}

/// The compaction instruction.
pub fn compact_instruction() -> &'static str {
    text(include_str!("compact_instruction.md"))
}

/// The wrapper a compacted context starts with.
pub fn compact_context() -> &'static str {
    text(include_str!("compact_context.md"))
}
