//! The loop's own configuration values.

/// Values a turn runs with.
#[derive(Debug, Clone)]
pub struct Settings {
    pub max_tool_rounds: Option<usize>,
    pub max_output_tokens: Option<u32>,
    pub scrooge: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_tool_rounds: None,
            max_output_tokens: None,
            scrooge: false,
        }
    }
}
