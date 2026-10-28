//! Post: one change of session state, with a receipt.

use std::path::PathBuf;

use super::MemoScope;

/// A one-shot change.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Post {
    Session {
        session: i64,
        write: SessionWrite,
    },
    Tool(ToolWrite),
    Server(ServerWrite),
    Init {
        cwd: PathBuf,
        model: Option<ModelId>,
        name: Option<String>,
        profile: Option<String>,
    },
}

/// What carrying out a post left behind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PostDone {
    Done,
    Created(i64),
    Busy,
}

/// A model addressed as `<provider>:<id>`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelId {
    pub provider: String,
    pub model: String,
}

impl ModelId {
    pub fn addressed(&self) -> String {
        format!("{}:{}", self.provider, self.model)
    }
}

/// One change to a session.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum SessionWrite {
    Name(Option<String>),
    Cwd(String),
    Switch(ModelId),
    Profile(String),
    TodoOn { call_id: String },
    TodoOff { call_id: String },
    Delete,
    Interrupt,
    Rewind { turn: i64, block: i64 },
    Unwatch,
}

/// One change to the server's own state.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ServerWrite {
    DefaultModel(ModelId),
    Reload,
}

/// One change to a tool's data.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ToolWrite {
    Memo(MemoWrite),
}

/// What a memo write does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoWriteOp {
    Add,
    Mod,
    Del,
}

/// One memo write.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct MemoWrite {
    pub op: MemoWriteOp,
    pub scope: MemoScope,
    pub anchor: Option<i64>,
    pub name: String,
    pub content: Option<String>,
}
