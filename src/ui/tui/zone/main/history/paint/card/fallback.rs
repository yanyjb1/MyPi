//! Unknown tools: the call as JSON, then whatever came back.

use super::ToolRenderer;

pub(crate) struct Fallback;

impl ToolRenderer for Fallback {}
