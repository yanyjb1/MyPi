use std::io::BufRead;
use std::sync::atomic::AtomicBool;

use anyhow::Result;

use crate::server::agent::request::content::BlockSink;
use crate::server::agent::request::message::AssistantMessage;
use crate::server::agent::request::protocols::openai::completion::compat::Resolved;
use crate::server::agent::request::protocols::openai::completion::{body_json, messages_to_wire};
use crate::server::agent::request::protocols::openai::stream::decode_completion;
use crate::server::agent::request::protocols::{Protocol, ProtocolRequest, WireRequest};
use crate::server::agent::request::transport::client::Client;

pub struct Deepseek {
    compat: Resolved,
}

impl Deepseek {
    pub fn new(_model: &str) -> Self {
        Self {
            compat: Resolved::conservative(),
        }
    }
}

impl Protocol for Deepseek {
    fn request(&self, client: &Client, req: &ProtocolRequest) -> Result<WireRequest> {
        let messages = messages_to_wire(&req.messages, &self.compat);
        let tools = serde_json::to_value(&req.tools).expect("tool serialization is infallible");
        let body = body_json(
            &req.model,
            req.max_output_tokens,
            req.stream,
            &messages,
            &tools,
            &self.compat,
            req.session_id.as_deref(),
        );
        Ok(WireRequest {
            url: format!("{}/chat/completions", client.base_url.trim_end_matches('/')),
            headers: vec![(
                "Authorization".to_string(),
                format!("Bearer {}", client.api_key),
            )],
            body,
        })
    }

    fn decode(
        &self,
        reader: Box<dyn BufRead>,
        sink: &mut dyn BlockSink,
        cancel: &AtomicBool,
    ) -> Result<AssistantMessage> {
        decode_completion(reader, &self.compat, sink, cancel)
    }

    fn compat_json(&self) -> Option<String> {
        None
    }
}
