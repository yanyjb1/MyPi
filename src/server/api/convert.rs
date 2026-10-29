//! Storage shapes into the JSON the front end reads.

use serde_json::{Value, json};

use crate::server::agent::tools;
use crate::server::storage;

pub(crate) fn block(value: storage::Block) -> Value {
    json!({
        "id": value.id,
        "kind": value.kind.as_str(),
        "content": value.content,
        "prefix": value.prefix,
        "suffix": value.suffix,
        "usage_total": value.usage_total,
        "usage_prompt": value.usage_prompt,
        "usage_completion": value.usage_completion,
        "usage_cached": value.usage_cached,
        "usage_cache_write": value.usage_cache_write,
        "usage_reasoning": value.usage_reasoning,
        "call_id": value.call_id,
        "name": value.name,
        "args": value.args,
        "intent": value.intent,
        "text": value.text,
        "first": value.first,
        "ok": value.ok,
        "result": value.result,
        "memo": value.memo,
        "duration_ms": value.duration_ms,
        "ts": value.ts,
    })
}

pub(crate) fn new_block(value: storage::NewBlock) -> Value {
    json!({
        "kind": value.kind.as_str(),
        "content": value.content,
        "prefix": value.prefix,
        "suffix": value.suffix,
        "usage_total": value.usage_total,
        "usage_prompt": value.usage_prompt,
        "usage_completion": value.usage_completion,
        "usage_cached": value.usage_cached,
        "usage_cache_write": value.usage_cache_write,
        "usage_reasoning": value.usage_reasoning,
        "call_id": value.call_id,
        "name": value.name,
        "args": value.args,
        "intent": value.intent,
        "text": value.text,
        "first": value.first,
        "ok": value.ok,
        "result": value.result,
        "memo": value.memo,
        "spill": value.spill.map(spill),
        "duration_ms": value.duration_ms,
        "ts": value.ts,
    })
}

pub(crate) fn session_meta(value: storage::SessionMeta) -> Value {
    json!({
        "id": value.id,
        "name": value.name,
        "started_at": value.started_at,
        "cwd": value.cwd,
        "model_provider": value.model_provider,
        "model": value.model,
        "profile": value.profile,
        "todo_on": value.todo_on,
    })
}

pub(crate) fn todo_phase(value: tools::TodoPhase) -> Value {
    json!({
        "name": value.name,
        "tasks": value.tasks.into_iter().map(todo_task).collect::<Vec<_>>(),
    })
}

pub(crate) fn usage(value: storage::RoundUsage) -> Value {
    json!({
        "prompt_tokens": value.prompt_tokens,
        "completion_tokens": value.completion_tokens,
        "cached_tokens": value.cached_tokens,
        "cache_write_tokens": value.cache_write_tokens,
    })
}

pub(crate) fn cut(value: storage::Cut) -> Value {
    match value {
        storage::Cut::Boundary { turn } => json!({"cut": "boundary", "turn": turn}),
        storage::Cut::Rescued { turn } => json!({"cut": "rescued", "turn": turn}),
        storage::Cut::Retyped { turn } => json!({"cut": "retyped", "turn": turn}),
    }
}

fn todo_task(value: tools::TodoTask) -> Value {
    json!({
        "content": value.content,
        "status": todo_status(value.status),
        "blocker": value.blocker,
    })
}

fn todo_status(value: tools::TodoStatus) -> &'static str {
    match value {
        tools::TodoStatus::Pending => "pending",
        tools::TodoStatus::InProgress => "in_progress",
        tools::TodoStatus::Done => "done",
        tools::TodoStatus::Blocked => "blocked",
        tools::TodoStatus::Abandoned => "abandoned",
    }
}

fn spill(value: storage::Spill) -> Value {
    json!({"name": value.name, "content": value.content})
}
