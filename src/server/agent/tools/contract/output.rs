//! What a call hands back: its text, its memo, its effects, and where its full text went.

use std::path::PathBuf;

use serde_json::Value;

use crate::server::agent::request::contract::Spill;

/// One finished call.
#[derive(Debug, Clone)]
pub struct ToolResult {
    pub call_id: String,
    pub ok: bool,
    pub result: String,
    /// Why the call failed, when it failed by being cut short.
    pub reason: Option<String>,
    pub memo: Option<String>,
    pub spill: Option<Spill>,
    pub duration_ms: u64,
    pub effects: Vec<Effect>,
    pub stop: bool,
}

/// One side effect the loop turns into rows, anchored on the result block.
#[derive(Debug, Clone)]
pub enum Effect {
    Memo {
        op: MemoOp,
        name: String,
        content: Option<String>,
        ts: String,
    },
    Alias {
        op: AliasOp,
        name: String,
        command: Option<String>,
        ts: String,
    },
    Child {
        tools: Vec<String>,
        cwd: PathBuf,
        start: String,
        max_tool_calls: u32,
        /// Where the sub-session's final text is written; the name is fixed when it is spawned.
        answer: PathBuf,
    },
}

impl Effect {
    /// Fill the timestamp the front door stamps for every effect.
    pub fn stamp(&mut self, ts: &str) {
        match self {
            Effect::Memo { ts: at, .. } | Effect::Alias { ts: at, .. } => *at = ts.to_string(),
            Effect::Child { .. } => {}
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemoOp {
    Set,
    Delete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AliasOp {
    Add,
    Delete,
}

/// A tool's return, before the front door turns it into a result.
#[derive(Debug)]
pub struct ToolOutput {
    pub text: String,
    /// A failure the text alone cannot state: the call was cut short, and why.
    pub fail: Option<String>,
    pub memo: Option<Value>,
    pub effects: Vec<Effect>,
    pub stop: bool,
}

impl ToolOutput {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            fail: None,
            memo: None,
            effects: Vec::new(),
            stop: false,
        }
    }

    pub fn with_memo(text: impl Into<String>, memo: Value) -> Self {
        Self {
            text: text.into(),
            fail: None,
            memo: Some(memo),
            effects: Vec::new(),
            stop: false,
        }
    }

    pub fn effect(mut self, effect: Effect) -> Self {
        self.effects.push(effect);
        self
    }

    pub fn stop(mut self) -> Self {
        self.stop = true;
        self
    }

    /// The call did not finish: the text stays the output so far, the reason rides beside it.
    pub fn failure(mut self, reason: impl Into<String>) -> Self {
        self.fail = Some(reason.into());
        self
    }
}
