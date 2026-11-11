//! One session's request side: the model it asks, and the prefix every lap leads with.

use std::sync::Arc;

use crate::server::agent::request::contract::Transport;
use crate::server::agent::request::message::ToolDef;
use crate::server::agent::request::transport::Provider;

/// The model a session asks: the address, the endpoint behind it, and its output cap.
#[derive(Debug, Clone)]
pub struct Model {
    pub provider: String,
    pub model: String,
    pub endpoint: Provider,
    pub max_output_tokens: u32,
}

impl Model {
    pub fn transport(&self) -> Transport {
        Transport::new(self.provider.clone(), self.model.clone())
    }
}

/// What every lap of one session sends: the model, the system prompt, the manuals, the cut.
#[derive(Clone)]
pub struct Config {
    pub model: Model,
    pub system: String,
    /// The session's tool names; the profile gate reads the same list.
    pub tools: Vec<String>,
    /// The manuals built when the session was booked; a lap never rebuilds them.
    pub manuals: Arc<Vec<ToolDef>>,
    /// Those manuals serialized once with the session.
    pub tools_json: Arc<serde_json::Value>,
    /// The compaction instruction template; `{{focus}}` marks where the focus goes.
    pub compact: String,
}

impl Config {
    pub(crate) fn new(birth: &super::book::Birth) -> Self {
        let tools_json =
            serde_json::to_value(birth.manuals.as_ref()).expect("tool manuals serialize to JSON");
        Self {
            model: Model {
                provider: birth.provider.clone(),
                model: birth.model.clone(),
                endpoint: birth.endpoint.clone(),
                max_output_tokens: birth.max_output_tokens,
            },
            system: birth.system.clone(),
            tools: birth.tools.clone(),
            manuals: birth.manuals.clone(),
            tools_json: Arc::new(tools_json),
            compact: birth.compact.clone(),
        }
    }
}
