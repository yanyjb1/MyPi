//! Storage — the single holder of state: `schema`, `shape`, `decode`, `writer`, `reader`, `delete`.
//! There are no reads here: a caller writes its own query and decodes the rows.

pub mod decode;
pub(crate) mod delete;
pub mod reader;
pub mod schema;
pub mod shape;
pub(crate) mod write;
pub(crate) mod writer;

/// The sessions database's file name.
pub const DB_FILE: &str = "history.db";

/// Where the sessions database lives by default: the platform's data directory plus [`DB_FILE`].
pub fn default_path() -> std::path::PathBuf {
    crate::server::platform::env::data_dir().join(DB_FILE)
}

pub use delete::rewind::{Cut, Target};
pub use reader::Reader;
pub use schema::SCHEMA_VERSION;
pub use shape::{
    AliasOp, Artifact, Block, BlockKind, ContentKey, Effect, EffectKind, Landed, MemoOp, MemoScope,
    NewBlock, Relay, RequestMeta, RoundUsage, SessionChange, SessionMeta, SessionRef, Spill,
    Submit, TurnRow, base36, parse_base36,
};
pub use write::compaction::BORROWED;
pub use writer::Writer;
