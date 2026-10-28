//! Wire data: the block, the session row, and the todo list, as both sides see them.

use serde::{Deserialize, Serialize};

/// What a block is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Hash, Serialize, Deserialize)]
pub enum BlockKind {
    #[default]
    User,
    Wake,
    Assistant,
    Reasoning,
    ToolRequest,
    ToolResult,
}

/// One block that has not landed yet.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct NewBlock {
    pub kind: BlockKind,
    pub content: Option<String>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub usage_total: Option<u64>,
    pub usage_prompt: Option<u64>,
    pub usage_completion: Option<u64>,
    pub usage_cached: Option<u64>,
    pub usage_cache_write: Option<u64>,
    pub usage_reasoning: Option<u64>,
    pub call_id: Option<String>,
    pub name: Option<String>,
    pub args: Option<String>,
    pub intent: Option<String>,
    pub text: Option<String>,
    pub first: Option<bool>,
    pub ok: Option<bool>,
    pub result: Option<String>,
    pub memo: Option<String>,
    pub spill: Option<Spill>,
    pub duration_ms: Option<i64>,
    pub ts: Option<String>,
}

/// The whole text of an oversized tool output.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Spill {
    pub name: String,
    pub content: String,
}

/// One landed block.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub id: i64,
    pub kind: BlockKind,
    pub content: Option<String>,
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    pub usage_total: Option<u64>,
    pub usage_prompt: Option<u64>,
    pub usage_completion: Option<u64>,
    pub usage_cached: Option<u64>,
    pub usage_cache_write: Option<u64>,
    pub usage_reasoning: Option<u64>,
    pub call_id: Option<String>,
    pub name: Option<String>,
    pub args: Option<String>,
    pub intent: Option<String>,
    pub text: Option<String>,
    pub first: Option<bool>,
    pub ok: Option<bool>,
    pub result: Option<String>,
    pub memo: Option<String>,
    pub duration_ms: Option<i64>,
    pub ts: Option<String>,
}

/// The id-claim recipe: both sides must hash the same fields.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentKey<'a> {
    pub kind: BlockKind,
    pub content: Option<&'a str>,
    pub call_id: Option<&'a str>,
    pub name: Option<&'a str>,
    pub args: Option<&'a str>,
    pub result: Option<&'a str>,
    pub text: Option<&'a str>,
}

impl ContentKey<'_> {
    pub fn digest(&self) -> u64 {
        use std::hash::{Hash as _, Hasher as _};
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.hash(&mut hasher);
        hasher.finish()
    }
}

impl Block {
    pub fn content_key(&self) -> ContentKey<'_> {
        ContentKey {
            kind: self.kind,
            content: self.content.as_deref(),
            call_id: self.call_id.as_deref(),
            name: self.name.as_deref(),
            args: self.args.as_deref(),
            result: self.result.as_deref(),
            text: self.text.as_deref(),
        }
    }
}

impl NewBlock {
    pub fn content_key(&self) -> ContentKey<'_> {
        ContentKey {
            kind: self.kind,
            content: self.content.as_deref(),
            call_id: self.call_id.as_deref(),
            name: self.name.as_deref(),
            args: self.args.as_deref(),
            result: self.result.as_deref(),
            text: self.text.as_deref(),
        }
    }
}

/// A session row.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionMeta {
    pub id: i64,
    pub name: Option<String>,
    pub started_at: String,
    pub cwd: Option<String>,
    pub model_provider: Option<String>,
    pub model: Option<String>,
    pub profile: Option<String>,
    pub todo_on: bool,
}

/// One entry on the todo list.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoTask {
    pub content: String,
    pub status: TodoStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocker: Option<String>,
}

/// How far one todo task has got.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TodoStatus {
    Pending,
    InProgress,
    Done,
    Blocked,
    Abandoned,
}

/// One named group of todo tasks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TodoPhase {
    pub name: String,
    pub tasks: Vec<TodoTask>,
}

/// Display name: the stored name, else `MM-DD:HH-MMSS+first 7 chars`.
pub fn display_name(meta: &SessionMeta, first_user: Option<&str>) -> String {
    if let Some(n) = &meta.name {
        return n.clone();
    }
    let prefix = first_user
        .map(|s| s.chars().take(7).collect::<String>())
        .unwrap_or_default();
    let t: Vec<char> = meta.started_at.chars().collect();
    let part = |a: usize, b: usize| t[a..b].iter().collect::<String>();
    if t.len() >= 19 {
        format!(
            "{}-{}:{}-{}{}+{}",
            part(5, 7),
            part(8, 10),
            part(11, 13),
            part(14, 16),
            part(17, 19),
            prefix
        )
    } else {
        format!("?+{prefix}")
    }
}
