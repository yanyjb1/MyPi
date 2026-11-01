//! Configuration: the key table, the two files, profiles, and the values they carry.

pub mod paths;
pub mod profile;
pub mod prompts;

mod load;
mod model;
mod notice;
mod registry;
mod schema;
mod write;

pub use load::{Config, Effective};
pub use model::{Cost, Currency, ModelEntry, ModelsConfig, Provider, ResolvedModel};
pub use notice::ConfigNotice;
pub use paths::ProjectDirs;
pub use profile::{DEFAULT_PROFILE, OMP_PROFILE, ResolvedProfile};
pub use registry::{BoolOrInt, DefaultValue, Key, Kind, Value, model_key, provider_key, template};
pub use schema::*;

/// The browser's launch values; the web domain reads them when it starts one.
#[derive(Debug, Clone, Default)]
pub struct BrowserOptions {
    pub bin: Option<std::path::PathBuf>,
    pub port: Option<u16>,
    pub profile_dir: Option<std::path::PathBuf>,
    pub headless: bool,
    pub user_agent: Option<String>,
    pub extra_args: Vec<String>,
}
