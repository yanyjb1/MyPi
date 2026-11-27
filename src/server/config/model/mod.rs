//! The `models.` area: the one global `models.yml`, parsed and queried.

mod effort;
mod entry;
mod parse;
mod protocol;
mod query;
mod types;

pub use effort::Effort;
pub use parse::parse;
pub use protocol::{ProtocolError, ProtocolId};
pub use query::is_model_key;
pub use types::{Cost, Currency, ModelEntry, ModelsConfig, Provider, ResolvedModel};
