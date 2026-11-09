//! Profile types.

pub const DEFAULT_PROFILE: &str = "default";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProfile {
    pub system: String,
    pub tools: Option<Vec<String>>,
    pub compact_instruction: String,
}
