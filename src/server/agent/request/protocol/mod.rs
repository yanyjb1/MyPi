//! The wire: protocol ids, the shapes one call takes, and what a decode hands back.

pub mod anthropic;
pub mod blocks;
pub mod openai;
pub mod sse;
pub mod stitch;
pub mod stream;

use crate::server::platform::stop::Stop;
use std::fmt;
use std::io::BufRead;
use std::sync::Arc;

use anyhow::Result;

use super::contract::Block;
use super::live::Stream;
use super::message::{AssistantMessage, DEFAULT_EMPTY_REPLY, ToolDef, WireMessage};

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
            Self::Completion => Box::new(openai::completion::Completion::new(base_url, model)),
            Self::Deepseek => Box::new(openai::deepseek::Deepseek::new()),
            Self::Responses => Box::new(openai::responses::Responses::new()),
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
    pub tools: Arc<Vec<ToolDef>>,
    /// The same manuals, already serialized by the session that owns them.
    pub tools_json: Arc<serde_json::Value>,
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

    /// How long a silent stream may last; the worker child enforces it.
    fn budget(&self) -> stream::Budget;

    /// Read the reply, emitting pieces as they arrive and each block as it closes.
    fn decode(
        &self,
        reader: Box<dyn BufRead>,
        out: &mut dyn Output,
        cancel: &Stop,
    ) -> Result<AssistantMessage>;
}

/// The endings an upstream declares when it finished on purpose: the model chose to stop.
const SILENT_ENDINGS: &[&str] = &["stop", "end_turn", "stop_sequence", "completed"];

/// The verdict on a reply that delivered neither text nor one whole call. A declared normal ending
/// is the model's own silence; anything else is the upstream breaking its side of the wire.
pub(crate) fn silence(finish: Option<&str>) -> std::result::Result<&'static str, String> {
    match finish {
        Some(ending) if SILENT_ENDINGS.contains(&ending) => Ok(DEFAULT_EMPTY_REPLY),
        Some(ending) => Err(format!(
            "upstream ended on `{ending}` and delivered no text and no whole call"
        )),
        None => Err(
            "upstream closed the stream without a finish reason and delivered no text and no whole call"
                .to_string(),
        ),
    }
}

/// The verdict on calls the model placed and the wire never closed into one object.
pub(crate) fn broken_calls(count: usize, finish: Option<&str>) -> String {
    let calls = if count == 1 { "call" } else { "calls" };
    let ending = match finish {
        Some(ending) => format!("ended on `{ending}`"),
        None => "no finish reason".to_string(),
    };
    format!(
        "upstream sent {count} tool {calls} whose arguments never closed into one object ({ending})"
    )
}

/// Land the fault as a stored block, so the turn says what happened, and make the error that ends
/// the lap. The reader sees the machine speaking, never the model.
pub(crate) fn fault(out: &mut dyn Output, text: String) -> anyhow::Error {
    out.block(blocks::assistant(&format!("SYSTEM: ({text})")));
    anyhow::anyhow!(text)
}
