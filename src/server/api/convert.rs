//! The API boundary: internal storage and agent shapes become wire contract shapes here.

use crate::ipc::contract as wire;
use crate::server::agent::tools;
use crate::server::storage;

pub(crate) fn block(value: storage::Block) -> wire::Block {
    wire::Block {
        id: value.id,
        kind: block_kind(value.kind),
        content: value.content,
        prefix: value.prefix,
        suffix: value.suffix,
        usage_total: value.usage_total,
        usage_prompt: value.usage_prompt,
        usage_completion: value.usage_completion,
        usage_cached: value.usage_cached,
        usage_cache_write: value.usage_cache_write,
        usage_reasoning: value.usage_reasoning,
        call_id: value.call_id,
        name: value.name,
        args: value.args,
        intent: value.intent,
        text: value.text,
        first: value.first,
        ok: value.ok,
        result: value.result,
        memo: value.memo,
        duration_ms: value.duration_ms,
        ts: value.ts,
    }
}

pub(crate) fn new_block(value: storage::NewBlock) -> wire::NewBlock {
    wire::NewBlock {
        kind: block_kind(value.kind),
        content: value.content,
        prefix: value.prefix,
        suffix: value.suffix,
        usage_total: value.usage_total,
        usage_prompt: value.usage_prompt,
        usage_completion: value.usage_completion,
        usage_cached: value.usage_cached,
        usage_cache_write: value.usage_cache_write,
        usage_reasoning: value.usage_reasoning,
        call_id: value.call_id,
        name: value.name,
        args: value.args,
        intent: value.intent,
        text: value.text,
        first: value.first,
        ok: value.ok,
        result: value.result,
        memo: value.memo,
        spill: value.spill.map(spill),
        duration_ms: value.duration_ms,
        ts: value.ts,
    }
}

pub(crate) fn session_meta(value: storage::SessionMeta) -> wire::SessionMeta {
    wire::SessionMeta {
        id: value.id,
        name: value.name,
        started_at: value.started_at,
        cwd: value.cwd,
        model_provider: value.model_provider,
        model: value.model,
        profile: value.profile,
        todo_on: value.todo_on,
    }
}

pub(crate) fn todo_phase(value: tools::TodoPhase) -> wire::TodoPhase {
    wire::TodoPhase {
        name: value.name,
        tasks: value
            .tasks
            .into_iter()
            .map(|task| wire::TodoTask {
                content: task.content,
                status: todo_status(task.status),
                blocker: task.blocker,
            })
            .collect(),
    }
}

pub(crate) fn turn_usage(value: storage::RoundUsage) -> wire::TurnUsage {
    wire::TurnUsage {
        prompt_tokens: value.prompt_tokens,
        completion_tokens: value.completion_tokens,
        cached_tokens: value.cached_tokens,
        reasoning_tokens: value.reasoning_tokens,
        cache_write_tokens: value.cache_write_tokens,
    }
}

fn block_kind(value: storage::BlockKind) -> wire::BlockKind {
    match value {
        storage::BlockKind::User => wire::BlockKind::User,
        storage::BlockKind::Wake => wire::BlockKind::Wake,
        storage::BlockKind::Assistant => wire::BlockKind::Assistant,
        storage::BlockKind::Reasoning => wire::BlockKind::Reasoning,
        storage::BlockKind::ToolRequest => wire::BlockKind::ToolRequest,
        storage::BlockKind::ToolResult => wire::BlockKind::ToolResult,
    }
}

fn todo_status(value: tools::TodoStatus) -> wire::TodoStatus {
    match value {
        tools::TodoStatus::Pending => wire::TodoStatus::Pending,
        tools::TodoStatus::InProgress => wire::TodoStatus::InProgress,
        tools::TodoStatus::Done => wire::TodoStatus::Done,
        tools::TodoStatus::Blocked => wire::TodoStatus::Blocked,
        tools::TodoStatus::Abandoned => wire::TodoStatus::Abandoned,
    }
}

fn spill(value: storage::Spill) -> wire::Spill {
    wire::Spill {
        name: value.name,
        content: value.content,
    }
}
