//! Post: the thirteen write names, their payloads, and the shared write shapes.

use std::path::PathBuf;

use super::call::memo::MemoScope;

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

/// What a rewind left behind.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "cut", rename_all = "snake_case")]
pub enum Cut {
    Boundary { turn: i64 },
    Rescued { turn: i64 },
    Retyped { turn: Option<i64> },
}

/// A session write, one per session-scoped post name.
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

/// A server write, one per server-scoped post name.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum ServerWrite {
    DefaultModel(ModelId),
    Reload,
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

/// Request payloads.
pub mod ask {
    use super::*;

    /// `post.init`.
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct Init {
        pub cwd: PathBuf,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub model: Option<ModelId>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub profile: Option<String>,
    }

    /// `post.name`.
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct Name {
        pub session: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        pub name: Option<String>,
    }

    /// `post.cwd`.
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct Cwd {
        pub session: i64,
        pub path: String,
    }

    /// `post.switch`.
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct Switch {
        pub session: i64,
        pub model: ModelId,
    }

    /// `post.profile`.
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct Profile {
        pub session: i64,
        pub profile: String,
    }

    /// `post.todo_on` / `post.todo_off`.
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct Todo {
        pub session: i64,
        pub call_id: String,
    }

    /// `post.delete` / `post.interrupt` / `post.unwatch`.
    #[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
    pub struct Session {
        pub session: i64,
    }

    /// `post.rewind`.
    #[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
    pub struct Rewind {
        pub session: i64,
        pub turn: i64,
        pub block: i64,
    }

    /// `post.default_model`.
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct DefaultModel {
        pub model: ModelId,
    }
}

/// Reply payloads.
pub mod reply {
    use super::*;

    /// `post.init` → `{session}`.
    #[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
    pub struct Init {
        pub session: i64,
    }

    /// `post.cwd` → `{cwd}`.
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct Cwd {
        pub cwd: PathBuf,
    }

    /// `post.rewind` → `{cut}`.
    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    pub struct Rewind {
        pub cut: super::Cut,
    }
}
