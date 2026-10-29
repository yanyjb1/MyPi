//! Storage — the single holder of state: `schema`, `shape`, `decode`, `writer`, `reader`, `delete`.
//! Writes go through `writer`; reads go through `reader`, which owns every query and every decode.

pub(crate) mod decode;
pub(crate) mod delete;
mod path;
pub mod reader;
pub mod schema;
pub mod shape;
pub(crate) mod write;
pub(crate) mod writer;

pub use delete::rewind::{Cut, Target};
pub use path::{DB_FILE, default_path};
pub use reader::Reader;
pub use schema::SCHEMA_VERSION;
pub use shape::{
    Block, BlockKind, Landed, NewBlock, Ref, Relay, RequestMeta, RoundUsage, Row, Submit, Table,
    base36, parse_base36,
};
pub use writer::Writer;
