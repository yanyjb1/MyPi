//! The wire: protocol ids, the shapes one call takes, and what a decode hands back.

pub mod anthropic;
pub mod blocks;
pub mod completion;
pub mod deepseek;
pub mod responses;
pub mod sse;
pub mod stitch;

use std::fmt;
use std::io::BufRead;
use std::sync::atomic::AtomicBool;

use anyhow::Result;

use super::contract::Block;
use super::live::Stream;
use super::message::{AssistantMessage, ToolDef, WireMessage};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolId {
    Completion,
    Deepseek,
    Responses,
    Anthropic,
}

impl ProtocolId {
    /// The one spelling of a protocol id; `models.yml`'s `api` value must match it.
    pub fn parse(raw: &str) -> Result<Self, ProtocolError> {
        Ok(match raw {
            "openai-completion" => Self::Completion,
            "openai-deepseek" => Self::Deepseek,
            "openai-responses" => Self::Responses,
            "anthropic-messages" => Self::Anthropic,
            other => return Err(ProtocolError(other.to_string())),
        })
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Completion => "openai-completion",
            Self::Deepseek => "openai-deepseek",
            Self::Responses => "openai-responses",
            Self::Anthropic => "anthropic-messages",
        }
    }

    /// Build one protocol for one endpoint. Compatibility is settled here, once.
    pub fn build(self, base_url: &str, model: &str) -> Box<dyn Protocol> {
        match self {
            Self::Completion => Box::new(completion::Completion::new(base_url, model)),
            Self::Deepseek => Box::new(deepseek::Deepseek::new()),
            Self::Responses => Box::new(responses::Responses::new()),
            Self::Anthropic => Box::new(anthropic::Messages::new()),
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

/// What one protocol needs to know about its host.
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub base_url: String,
    pub api_key: String,
    /// False sends no credential header at all.
    pub auth_header: bool,
}

/// Everything one call sends.
#[derive(Debug, Clone)]
pub struct ProtocolRequest {
    pub model: String,
    pub max_output_tokens: u32,
    pub stream: bool,
    pub messages: Vec<WireMessage>,
    pub tools: Vec<ToolDef>,
    pub session_id: Option<String>,
}

/// One assembled HTTP call.
#[derive(Debug, Clone)]
pub struct WireRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: serde_json::Value,
}

/// Where a decode puts what it hears. The caller routes each half onward.
pub trait Output {
    fn stream(&mut self, piece: Stream);
    fn block(&mut self, block: Block);
}

pub trait Protocol: Send + Sync {
    /// Serialize one call for this host.
    fn request(&self, endpoint: &Endpoint, req: &ProtocolRequest) -> Result<WireRequest>;

    /// Read the reply, emitting pieces as they arrive and each block as it closes.
    fn decode(
        &self,
        reader: Box<dyn BufRead>,
        out: &mut dyn Output,
        cancel: &AtomicBool,
    ) -> Result<AssistantMessage>;
}
