//! Configuration: the key table, the two files, profiles, and the values they carry.

pub mod paths;
pub mod profile;
pub mod prompts;
#[cfg(feature = "browser")]
pub mod rules;

mod load;
mod model;
mod notice;
mod registry;
mod schema;
mod write;

#[cfg(feature = "browser")]
pub use load::BrowserOptions;
pub use load::{Config, Effective};
pub use model::{
    Cost, Currency, Effort, ModelEntry, ModelsConfig, ProtocolError, ProtocolId, Provider,
    ResolvedModel,
};
pub use notice::ConfigNotice;
pub use profile::{DEFAULT_PROFILE, ResolvedProfile};
pub use registry::{BoolOrInt, DefaultValue, Key, Kind, Value, model_key, provider_key, template};
pub use schema::*;
