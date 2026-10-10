//! Shapes: what one database row becomes in memory — the block, the round, the session, the
//! request archive, an artifact, and what is submitted to the write inlet.

/// What a block is (the `type` column, typed).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum BlockKind {
    /// The user's own message.
    #[default]
    User,
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
    /// The whole text of an oversized tool output, riding the block until it has an id.
    pub spill: Option<Spill>,
    /// How long the tool took (tool result).
    pub duration_ms: Option<i64>,
    /// When this block landed.
    pub ts: Option<String>,
}

/// The full text of an oversized tool output, waiting for its block's id: one block, one
/// artifact, the block's id is the artifact's id.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Spill {
    pub name: String,
    pub content: String,
}

/// The base-36 spelling of a block id: what a citation names. The id is frozen from the moment
/// the block lands, so a citation stays valid across rewinds and forks.
pub fn base36(mut n: i64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n <= 0 {
        return "0".to_string();
    }
    let mut out = Vec::new();
    while n > 0 {
        out.push(DIGITS[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).expect("the digits are ASCII")
}

/// Read a citation's spelling back. `None` = not a block id.
pub fn parse_base36(s: &str) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    i64::from_str_radix(s, 36).ok()
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
    pub model_provider: Option<String>,
    pub model: Option<String>,
    /// Whether the todo tool is on for this session. Read by the api's list
    /// initializer; the list itself is replayed from the tool calls.
    pub todo_on: bool,
}

/// One round's usage, stored per round. Only tokens are stored: whoever draws the bill turns them
/// into money.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RoundUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    /// The cache-read part of `prompt_tokens`.
    pub cached_tokens: u64,
    /// The reasoning part of `completion_tokens` (thinking models only).
    pub reasoning_tokens: u64,
    /// The part of `prompt_tokens` written into the cache.
    pub cache_write_tokens: u64,
}

/// The conversation-invariant part of one request (the request archive). It never enters the
/// message list; one row per distinct archive (deduplicated by `hash`).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct RequestMeta {
    pub model: String,
    /// Provider name. The same model id can live under two providers; the
    /// bill must tell them apart.
    pub provider: String,
    /// The wire protocol id, e.g. `openai-chat-completions`.
    pub protocol: String,
    /// The endpoint base (`https://host/v1`). Never a secret.
    pub base_url: String,
    /// The system prompt **verbatim**.
    pub system: String,
    /// The tool manuals **verbatim** (the serialized `tools` array).
    pub tools_json: String,
    /// The token ceiling this request carried.
    pub max_tokens: u32,
    /// The `compat:` block from `models.yml`, verbatim; `None` = none.
    pub compat_json: Option<String>,
}

/// One round (a `turns` row plus its request archive).
#[derive(Debug, Clone, PartialEq)]
pub struct TurnRow {
    pub turn_id: i64,
    /// The request archive's row id; `None` = this round carried no archive
    /// (an in-memory caller).
    pub request_id: Option<i64>,
    /// The blocks this round occupies, as `[start, end]` pairs in order (one
    /// interval per landed batch).
    pub blocks: Vec<[i64; 2]>,
    /// How the round ended: `"rewind"`, `"compaction"`,
    /// [`BORROWED`](super::write::compaction::BORROWED); `None` = not finished yet.
    pub stop_reason: Option<String>,
    /// This round's block volume (borrowed intervals included).
    pub bytes: i64,
    /// Usage; `None` = never received (died mid-flight, the gateway does not
    /// report, or a borrowed row).
    pub usage: Option<RoundUsage>,
    pub request: Option<RequestMeta>,
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

/// One oversized tool output. Its identity is the block it came from, so nothing maps a citation
/// to a row.
#[derive(Debug, Clone, PartialEq)]
pub struct Artifact {
    /// The block that produced it.
    pub block: i64,
    pub name: String,
    pub total_lines: i64,
    pub content: String,
}

/// A change to a session row.
#[derive(Debug, Clone, PartialEq)]
pub enum SessionChange {
    /// `/name` (`None` = clear it).
    Name(Option<String>),
    /// `/switch` (`None` = follow the configured default).
    Model(Option<(String, String)>),
    /// `/cdp`: the absolute path, and the round the history is anchored on
    /// (`None` = the session has no rounds yet, so only the session row lands).
    Cwd { turn: Option<i64>, path: String },
}

/// Which session a submission goes to, or the makings of a new one.
#[derive(Debug, Clone)]
pub enum SessionRef {
    /// Continue this session.
    Id(i64),
    /// Start a new one: when it starts and where it was launched (the caller
    /// reads the clock and knows the working directory).
    New { started_at: String, cwd: String },
}

/// What is handed to the write inlet: one thing at a time, each naming the
/// table it lands in.
#[derive(Debug, Clone)]
pub enum Submit {
    /// A session row on its own, before any message: the front end needs the id first.
    NewSession {
        started_at: String,
        cwd: String,
        /// This session's model (`provider`, bare id); `None` = follow the
        /// configured default.
        model: Option<(String, String)>,
        /// The display name; `None` = unnamed.
        name: Option<String>,
    },
    /// The user's message: opens a round and lands at once. `session` = `None`
    /// creates the session row first.
    UserMessage {
        session: SessionRef,
        prefix: Option<String>,
        text: String,
        suffix: Option<String>,
        /// When this message lands (the caller reads the clock).
        ts: Option<String>,
    },
    /// The blocks of one reply (or of one tool round): relayed at once, landed every `batch` tool
    /// rounds. `done` = these blocks are finished; an open one is still being written.
    Blocks {
        session: i64,
        blocks: Vec<NewBlock>,
        done: bool,
    },
    /// The round is over: settle it at once.
    RoundEnd {
        session: i64,
        stop_reason: Option<String>,
        usage: Option<RoundUsage>,
    },
    /// One request's archive, reported before the round sends it.
    Request {
        session: i64,
        meta: Box<RequestMeta>,
    },
    /// A change to the session row.
    Session {
        session: i64,
        change: SessionChange,
    },
    /// A compaction: the summary round plus the rounds borrowing an index.
    Compaction {
        session: i64,
        summary: NewBlock,
        request: Option<Box<RequestMeta>>,
        /// `(round, from)` — `from` = where the borrowed stretch starts
        /// (`None` = the whole round).
        kept: Vec<(i64, Option<i64>)>,
    },
}

/// What one `push` landed.
#[derive(Debug, Clone, Default)]
pub struct Landed {
    /// The session it created, when it created one.
    pub session: Option<i64>,
    /// The round it touched, when there is one.
    pub turn: Option<i64>,
    /// The blocks that just landed, ids included (empty when nothing landed).
    pub blocks: Vec<Block>,
}

/// What storage relays to the api while a round runs. Every piece names its session; the api
/// hands it to whoever asked for that session.
#[derive(Debug, Clone)]
pub enum Relay {
    /// The tail that has not landed yet (what a streaming reply produced).
    Tail { session: i64, blocks: Vec<NewBlock> },
    /// A batch just landed: its blocks, ids included, in order.
    Blocks { session: i64, blocks: Vec<Block> },
    /// The round ended; the reason as stored (`None` = died before the gateway answered), and the
    /// round id, so two surfaces each know which stretch just closed.
    Ended {
        session: i64,
        turn: Option<i64>,
        stop_reason: Option<String>,
    },
}
