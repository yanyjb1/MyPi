//! Request shapes: the four doors and what comes back. Every argument is a **parsed value** — no raw
//! command string crosses this file.

use crate::commands::Candidate;
use crate::server::storage::Block;

/// Number wording for the usage panel: English steps by 10^3, Chinese by 10^4.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageLang {
    /// `K` / `M` / `T`, three digits to a group.
    #[default]
    En,
    /// `万` / `亿` / `万亿`, four digits to a group.
    Zh,
}

/// A model addressed the way the config file addresses it: `<provider>:<id>`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ModelId {
    pub provider: String,
    pub model: String,
}

impl ModelId {
    /// The form the config speaks (`<provider>:<id>`).
    pub fn addressed(&self) -> String {
        format!("{}:{}", self.provider, self.model)
    }
}

/// A front end's own id, made up the moment it starts. The server never invents one: two ids are two
/// front ends, and one can stop watching without taking another's stream down.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Viewer(pub i64);

/// A one-shot question: ask, answer, done.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Call {
    /// The usage bill for one session (tokens per model, priced here).
    Usage { session: i64, lang: UsageLang },
    /// The resume picker's rows (metadata + preview + size).
    Sessions {
        /// Only sessions that worked under this directory; `None` = all.
        under: Option<std::path::PathBuf>,
    },
    /// A session's first frame: the last `limit` blocks of its viewport.
    Tail { session: i64, limit: usize },
    /// Older blocks for a sliding window.
    Older {
        session: i64,
        before: i64,
        limit: usize,
    },
    /// Newer blocks after the window dropped some.
    Newer {
        session: i64,
        after: i64,
        limit: usize,
    },
    /// The session's whole todo list, replayed from its own tool calls.
    Todo { session: i64 },
    /// The session's metadata as it stands now, plus the working directory
    /// that lives in memory (the model's `cd` moves it, nothing writes it).
    Meta { session: i64 },
    /// The two completion pools a surface asks for once, at startup: model ids
    /// and profile names. Paths are not here — a surface scans those itself.
    Pools,
}

/// A one-shot change of session state. A change carries no body; creating a session is the one
/// exception ([`Post::Init`]).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Post {
    /// `/name` — the session's display name (`None` clears it).
    Name { session: i64, name: Option<String> },
    /// `/cdp` — migrate the working directory. The path is the text the user
    /// typed; it is resolved against the driver's live cwd.
    Cwd { session: i64, path: String },
    /// `/switch` — this session's model, written to the session row.
    Switch { session: i64, id: ModelId },
    /// `/model` — the configured default; takes effect on the next start.
    DefaultModel { id: ModelId },
    /// `/profile` — which system prompt and tool roster the session runs.
    Profile { session: i64, profile: String },
    /// Create a session before any message; the front end needs the id.
    Init {
        /// Where the session starts: the front end read its own working
        /// directory and hands it over (the server does not guess it).
        cwd: std::path::PathBuf,
        /// This session's model; `None` = follow the configured default.
        model: Option<ModelId>,
        /// The display name; `None` = unnamed.
        name: Option<String>,
        /// Which profile to run; `None` = the config's active one.
        profile: Option<String>,
    },
    /// Remove a session and everything only it referenced.
    Delete { session: i64 },
    /// Kill the running round of one session (the Esc path). Fire-and-forget: the authoritative
    /// answer is the `StreamItem::Ended` that follows.
    Interrupt { session: i64 },
    /// Cut the conversation back to a block. The surface names the round it was showing: a block can
    /// be referenced by two rounds at once.
    Rewind { session: i64, turn: i64, block: i64 },
    /// Stop watching a session: this viewer's stream is dropped. Not watching is not an error.
    Unwatch { viewer: Viewer, session: i64 },
}

/// What carrying out a [`Post`] left behind. A change answers nothing; creating a session answers
/// with the new id.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PostDone {
    /// A session row was created; this is its id.
    Created(i64),
    /// The change landed; nothing to hand back.
    Done,
}

/// A subscription: either a claim on a session's **stream**, or the opening of
/// a stretch (a round, a compaction). Content arrives over time.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Subscribe {
    /// Ask for one session's stream, as this viewer. Asking twice as the same viewer replaces its
    /// old one; [`Post::Unwatch`] stops it.
    Watch { viewer: Viewer, session: i64 },
    /// Submit one user message: the prefix and the suffix were computed upstream (timestamp
    /// injection), the text is what the user typed. The three fields freeze as they arrive.
    Submit {
        /// The session this message opens a round in; the front end creates it first.
        session: i64,
        /// Injected before the body; `None` = nothing to inject.
        prefix: Option<String>,
        /// The user's own words, always clean.
        text: String,
        /// Injected after the body; `None` = nothing to inject.
        suffix: Option<String>,
    },
    /// Compact a session's context (`/compact`).
    Compact {
        session: i64,
        /// How much of the tail to keep, already parsed.
        retention: crate::server::api::subscribe::Retention,
        /// The free text after the number: what to emphasize in the summary
        /// (`None` = nothing).
        focus: Option<String>,
    },
}

/// What a [`Call`] answers with.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum Reply {
    /// Answer to [`Call::Usage`]: one row per model.
    Usage {
        lang: UsageLang,
        groups: Vec<crate::server::api::call::usage::ModelUsage>,
    },
    /// Answer to [`Call::Sessions`].
    SessionRows(Vec<crate::server::api::call::sessions::SessionRow>),
    /// Answer to the viewport calls: stored blocks in order, ids included, plus
    /// whether more exist in the direction asked.
    Blocks { blocks: Vec<Block>, more: bool },
    /// Answer to [`Call::Todo`]: the session's list as it stands.
    Todo(Vec<crate::server::agent::tools::TodoPhase>),
    /// Answer to [`Call::Meta`].
    Meta(crate::server::api::call::meta::SessionState),
    /// Answer to [`Call::Pools`].
    Pools(CandidatePools),
}

/// The two completion pools, in the shape every provider hands out.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CandidatePools {
    /// Model ids, addressed `<provider>:<id>`.
    pub models: Vec<Candidate>,
    /// Profile names from the roster.
    pub profiles: Vec<Candidate>,
}
