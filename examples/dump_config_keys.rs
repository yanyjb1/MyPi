//! Print every key the configuration accepts, exactly as a user would write it.
//!
//! The list is **not maintained by hand**. Each type is serialized with serde
//! and walked, so a field added, renamed or removed in `src/server/config/read/`
//! shows up here the next time this runs — there is no second copy to update.
//!
//! ```text
//! cargo run --example dump_config_keys
//! cargo run --example dump_config_keys -- --values-only
//! ```
//!
//! A value printed after `=` is the **built-in default** (`null` = nothing set,
//! i.e. "follow whatever else decides"). The three `profile.*` keys are the one
//! exception: they are not serde fields, so they are printed from their own tiny
//! list at the bottom.

use serde_json::Value;

use mypi::server::config::read::config::AppConfig;
use mypi::server::config::read::model::{ModelEntry, ModelsConfig, Provider};

fn main() {
    let values_only = std::env::args().any(|a| a == "--values-only");

    println!("# config.yaml");
    dump("config", &json_of(&AppConfig::default()), values_only);

    println!("\n# models.yml");
    let models = ModelsConfig {
        providers: [("<provider>".to_string(), Provider {
            models: vec![ModelEntry::default()],
            ..Provider::default()
        })]
        .into_iter()
        .collect(),
    };
    dump("models", &json_of(&models), values_only);

    println!("\n# the profile folder");
    for key in ["profile.system", "profile.compact", "profile.toolList"] {
        if values_only {
            println!("{key}");
        } else {
            println!("{key} = <a file or a name>");
        }
    }
}

/// Serialize anything the config layer parses, so the keys are the file's keys.
fn json_of<T: serde::Serialize>(v: &T) -> Value {
    serde_json::to_value(v).expect("the configuration types always serialize")
}

/// Walk a serialized default and print one line per leaf.
fn dump(prefix: &str, v: &Value, values_only: bool) {
    match v {
        Value::Object(map) => {
            for (k, val) in map {
                dump(&format!("{prefix}.{k}"), val, values_only);
            }
        }
        // `null` means "not set"; everything else prints its default.
        leaf => {
            if values_only {
                println!("{prefix}");
            } else {
                println!("{prefix} = {leaf}");
            }
        }
    }
}
