// The `api` value of `models.yml`: which wire protocol a provider speaks.

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolId {
    Completion,
    Deepseek,
}

impl ProtocolId {
    /// The one spelling of a protocol id; `models.yml`'s `api` value must match it.
    pub fn parse(raw: &str) -> Result<Self, ProtocolError> {
        Ok(match raw {
            "openai-completion" => Self::Completion,
            "openai-deepseek" => Self::Deepseek,
            other => return Err(ProtocolError(other.to_string())),
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completion => "openai-completion",
            Self::Deepseek => "openai-deepseek",
        }
    }
}

#[derive(Debug)]
pub struct ProtocolError(pub String);

impl fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown protocol id: {}", self.0)
    }
}

impl std::error::Error for ProtocolError {}
