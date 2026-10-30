//! Shapes: what one database row becomes in memory — the block, the round, the session, the
//! request anchor, an artifact, and what a caller submits.

/// What a block is (the `type` column, typed).
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default, Hash, serde::Serialize, serde::Deserialize,
)]
pub enum BlockKind {
    /// The user's own message.
    #[default]
    User,
    /// A system wake message.
    Wake,
    /// The model's reply (final).
    Assistant,
    /// The model's thinking, when the protocol reports it separately.
    Reasoning,
    /// One tool call the model asked for.
    ToolRequest,
    /// One tool call's result.
    ToolResult,
}

impl BlockKind {
    /// The column's spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            BlockKind::User => "user",
            BlockKind::Wake => "wake",
            BlockKind::Assistant => "assistant",
            BlockKind::Reasoning => "reasoning",
            BlockKind::ToolRequest => "tool_request",
            BlockKind::ToolResult => "tool_result",
        }
    }

    /// Read the column back. An unknown tag is a broken row: error, never
    /// silently guess.
    pub fn parse(raw: &str) -> anyhow::Result<Self> {
        Ok(match raw {
            "user" => BlockKind::User,
            "wake" => BlockKind::Wake,
            "assistant" => BlockKind::Assistant,
            "reasoning" => BlockKind::Reasoning,
            "tool_request" => BlockKind::ToolRequest,
            "tool_result" => BlockKind::ToolResult,
            other => anyhow::bail!("a block carries an unknown type: {other}"),
        })
    }
}

/// One block that has not landed yet: the row's columns, no id. Whoever cuts a message into
/// blocks fills it in; storage only writes it.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct NewBlock {
    pub kind: BlockKind,
    /// Body text (`user` / `assistant` / `reasoning`).
    pub content: Option<String>,
    /// Injection around a user message.
    pub prefix: Option<String>,
    pub suffix: Option<String>,
    /// The reply's usage (assistant blocks). Everything the gateway reports
    /// that touches the bill lands here — reasoning tokens included.
    pub usage_total: Option<u64>,
    pub usage_prompt: Option<u64>,
    pub usage_completion: Option<u64>,
    pub usage_cached: Option<u64>,
    pub usage_cache_write: Option<u64>,
    pub usage_reasoning: Option<u64>,
    /// The pairing key of a tool request and its result.
    pub call_id: Option<String>,
    /// The tool's name (tool request).
    pub name: Option<String>,
    /// The arguments the model sent, verbatim (tool request).
    pub args: Option<String>,
    /// The one line the model wrote about what it is doing (tool request).
    pub intent: Option<String>,
    /// The assistant message's own body, on the first call of a reply.
    pub text: Option<String>,
    /// Is this the reply's first tool call?
    pub first: Option<bool>,
    /// Did the tool succeed? (tool result)
    pub ok: Option<bool>,
    /// The text handed back to the model (tool result).
    pub result: Option<String>,
    /// Extra material for a front end (tool result); never enters a request.
    pub memo: Option<String>,
    /// How long the tool took (tool result).
    pub duration_ms: Option<i64>,
    /// When this block landed.
    pub ts: Option<String>,
}

impl NewBlock {
    /// The same columns, landed: the id comes back from the insert. The field list must stay in
    /// step with the columns.
    pub fn landed(self, id: i64) -> Block {
        Block {
            id,
            kind: self.kind,
            content: self.content,
            prefix: self.prefix,
            suffix: self.suffix,
            usage_total: self.usage_total,
            usage_prompt: self.usage_prompt,
            usage_completion: self.usage_completion,
            usage_cached: self.usage_cached,
            usage_cache_write: self.usage_cache_write,
            usage_reasoning: self.usage_reasoning,
            call_id: self.call_id,
            name: self.name,
            args: self.args,
            intent: self.intent,
            text: self.text,
            first: self.first,
            ok: self.ok,
            result: self.result,
            memo: self.memo,
            duration_ms: self.duration_ms,
            ts: self.ts,
        }
    }
}

/// One landed block: the same columns plus its id.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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

/// A session row.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SessionMeta {
    pub id: i64,
    /// The name written by `/name`; `None` = unnamed.
    pub name: Option<String>,
    pub started_at: String,
    /// Where the last `/cdp` landed; `None` = never migrated.
    pub cwd: Option<String>,
    /// Which model this session is on (provider + id); `None` = follow the
    /// configured default.
    pub provider: Option<String>,
    pub model: Option<String>,
    /// The block that spawned this session; `None` = a main session.
    pub anchor: Option<i64>,
    /// The request archive this session reads its system prompt and tools from.
    pub meta: Option<i64>,
    /// Whether the todo tool is on for this session.
    pub todo_on: bool,
}

/// One round's usage, stored per round. Only tokens are stored: whoever draws the bill turns them
/// into money.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct RoundUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// The cache-read part of `prompt_tokens`.
    pub cached_tokens: u64,
    /// The part of `prompt_tokens` written into the cache.
    pub cache_write_tokens: u64,
}

/// The conversation-invariant part of one session anchor. It never enters the message list;
/// one row per distinct anchor, deduplicated by `hash`.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct RequestMeta {
    pub model: String,
    /// Provider name. The same model id can live under two providers; the
    /// bill must tell them apart.
    pub provider: String,
    /// The system prompt **verbatim**.
    pub system: String,
    /// The tool manuals **verbatim** (the serialized `tools` array).
    pub tools_json: String,
    /// The smallest prompt measurement seen under this model: never grows.
    pub baseline_prompt: Option<u64>,
}

/// One round (a `turns` row and its block intervals).
#[derive(Debug, Clone, PartialEq)]
pub struct TurnRow {
    pub turn_id: i64,
    /// The blocks this round occupies, as `[start, end]` pairs in order (one
    /// interval per landed batch).
    pub blocks: Vec<[i64; 2]>,
    /// How the round ended (`stop` / `interrupted` / `broken` / `killed`) or what the row is
    /// (`rewind` / `compaction` / `borrowed`); `None` = not finished yet.
    pub stop_reason: Option<String>,
    /// Usage; `None` = never received (died mid-flight, the gateway does not
    /// report, or a borrowed row).
    pub usage: Option<RoundUsage>,
}

impl TurnRow {
    /// The first block this round occupies (`None` = the index is empty).
    pub fn first_block(&self) -> Option<i64> {
        self.blocks.first().map(|[lo, _]| *lo)
    }

    /// The last block this round occupies.
    pub fn last_block(&self) -> Option<i64> {
        self.blocks.last().map(|[_, hi]| *hi)
    }
}

/// A row id: known now, or produced by an earlier row of the same submission.
#[derive(Debug, Clone, Copy)]
pub enum Ref {
    Known(i64),
    Batch(usize),
}

/// One statement: a table and the fields it writes. No row carries another table's data.
#[derive(Debug, Clone)]
pub enum Row {
    /// A new session row.
    Session {
        started_at: String,
        cwd: String,
        model: Option<(String, String)>,
        name: Option<String>,
        anchor: Option<Ref>,
        meta: Option<Ref>,
    },
    /// `sessions.name`.
    Rename { session: i64, name: Option<String> },
    /// `sessions.provider` + `sessions.model`.
    Switch {
        session: i64,
        model: Option<(String, String)>,
    },
    /// `sessions.meta`.
    Meta { session: i64, meta: Option<Ref> },
    /// `sessions.cwd`.
    MoveCwd { session: i64, cwd: String },
    /// `sessions.todo_on`.
    TodoOn { session: i64, on: bool },
    /// A new round row.
    Turn {
        stop_reason: Option<String>,
        usage: Option<RoundUsage>,
    },
    /// `turns.stop_reason` + the four usage columns.
    Settle {
        turn: Ref,
        stop_reason: Option<String>,
        usage: Option<RoundUsage>,
    },
    /// A session anchor, deduplicated by content.
    Request { meta: RequestMeta },
    /// Lower one anchor's prompt baseline; it never grows.
    Baseline { request: Ref, prompt: u64 },
    /// One block; its id joins its round's interval and byte count.
    Block { turn: Ref, block: NewBlock },
    /// One artifact row.
    Artifact {
        block: Ref,
        name: String,
        content: String,
        hash: String,
        reference: String,
    },
    /// One memo row.
    Memo {
        anchor: Ref,
        name: String,
        content: String,
        ts: String,
    },
    /// One alias row.
    Alias {
        anchor: Ref,
        name: String,
        command: String,
        ts: String,
    },
    /// One working-directory entry.
    Cwd {
        session: i64,
        turn: Ref,
        cwd: String,
    },
    /// One place in a session's list.
    List {
        session: i64,
        turn: Ref,
        is_compaction: bool,
    },
    /// Borrow a set of block intervals into an existing round.
    Borrow { turn: Ref, ranges: Vec<[i64; 2]> },
}

/// A table whose rows stand alone: deleting one leaves nothing dangling.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Table {
    Sessions,
    Turns,
    Artifacts,
    Memos,
    Aliases,
}

/// One submission: the rows of one transaction, in order.
#[derive(Debug, Clone, Default)]
pub struct Submit {
    pub rows: Vec<Row>,
}

/// What one submission landed: one id per row, `0` where the row has none.
#[derive(Debug, Clone, Default)]
pub struct Landed {
    pub ids: Vec<i64>,
}

/// What the writer tells a live surface once a round is over: where it sits, and the closed id
/// ranges its blocks occupy, in order and split where they are discontinuous.
#[derive(Debug, Clone)]
pub struct Structure {
    pub session: i64,
    pub turn: i64,
    pub ranges: Vec<[i64; 2]>,
}
