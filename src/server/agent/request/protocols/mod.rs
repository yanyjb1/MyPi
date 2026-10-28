pub mod anthropic;
pub mod openai;

use std::io::BufRead;
use std::sync::atomic::AtomicBool;

use anyhow::Result;

use crate::server::agent::request::content::BlockSink;
use crate::server::agent::request::message::{AssistantMessage, ToolDef, WireMessage};
use crate::server::agent::request::protocols::openai::completion::compat::Compat;
use crate::server::agent::request::transport::client::Client;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProtocolId {
    OpenAiCompletion,
    OpenAiDeepseek,
    OpenAiResponses,
    AnthropicMessages,
}

impl ProtocolId {
    pub fn parse(raw: &str) -> Result<Self, ProtocolError> {
        match raw {
            "openai-completion" => Ok(Self::OpenAiCompletion),
            "openai-deepseek" => Ok(Self::OpenAiDeepseek),
            "openai-responses" => Ok(Self::OpenAiResponses),
            "anthropic-messages" => Ok(Self::AnthropicMessages),
            other => Err(ProtocolError::Unknown(other.to_string())),
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::OpenAiCompletion => "openai-completion",
            Self::OpenAiDeepseek => "openai-deepseek",
            Self::OpenAiResponses => "openai-responses",
            Self::AnthropicMessages => "anthropic-messages",
        }
    }

    pub fn build(self, base_url: &str, model: &str, user: Option<&Compat>) -> Box<dyn Protocol> {
        match self {
            Self::OpenAiCompletion => {
                Box::new(openai::completion::Completion::new(base_url, model, user))
            }
            Self::OpenAiDeepseek => Box::new(openai::deepseek::Deepseek::new(model)),
            Self::OpenAiResponses => Box::new(openai::responses::Responses::new(model)),
            Self::AnthropicMessages => Box::new(anthropic::messages::Messages::new(model)),
        }
    }
}

#[derive(Debug)]
pub enum ProtocolError {
    Unknown(String),
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(id) => write!(f, "unknown protocol id: {id}"),
        }
    }
}

impl std::error::Error for ProtocolError {}

pub trait Protocol: Send + Sync {
    fn request(&self, client: &Client, req: &ProtocolRequest) -> Result<WireRequest>;
    fn decode(
        &self,
        reader: Box<dyn BufRead>,
        sink: &mut dyn BlockSink,
        cancel: &AtomicBool,
    ) -> Result<AssistantMessage>;
    fn compat_json(&self) -> Option<String>;
}

pub struct ProtocolRequest {
    pub model: String,
    pub max_output_tokens: u32,
    pub stream: bool,
    pub messages: Vec<WireMessage>,
    pub tools: Vec<ToolDef>,
    pub session_id: Option<String>,
}

pub struct WireRequest {
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: serde_json::Value,
}
