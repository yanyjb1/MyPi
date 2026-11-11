//! One session's request side: the model it asks, and the prefix every lap leads with.

use std::sync::Arc;

use crate::server::agent::request::contract::Transport;
use crate::server::agent::request::message::ToolDef;

/// What every lap of one session sends: the model, the system prompt, the manuals, the cut.
#[derive(Clone)]
pub struct Config {
    pub provider: String,
    pub model: String,
    pub system: String,
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
            provider: birth.provider.clone(),
            model: birth.model.clone(),
            system: birth.system.clone(),
            manuals: birth.manuals.clone(),
            tools_json: Arc::new(tools_json),
            compact: birth.compact.clone(),
        }
    }

    pub fn transport(&self) -> Transport {
        Transport::new(self.provider.clone(), self.model.clone())
    }
}
