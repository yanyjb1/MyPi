//! The key table: every setting's name, kind, default and comment.

mod macros;
mod table;
mod types;

pub(crate) use macros::keys;
pub use table::{
    config_keys, cost_fields, is_config_key, is_config_prefix, lookup, model_fields, model_key,
    model_suffixes, provider_fields, provider_key, template,
};
pub use types::{BoolOrInt, DefaultValue, Key, Kind, Value};
