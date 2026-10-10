//! The completion pools: model ids and profile names, asked for **once** at startup. Paths are the
//! surface's own business.

use crate::commands::Candidate;
use crate::server::api::types::CandidatePools;
use crate::server::config::Config;
use crate::server::config::read::profile;

/// Both pools, ready to use.
pub fn pools(cfg: &Config) -> CandidatePools {
    CandidatePools {
        models: models(cfg),
        profiles: profiles(cfg),
    }
}

/// Every declared model, addressed `<provider>:<id>` — the form `/model` and
/// `/switch` speak. The detail is the display name the config gave it.
fn models(cfg: &Config) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = cfg
        .models()
        .map(|(provider, entry)| Candidate {
            value: format!("{}:{}", provider, entry.id),
            detail: entry.display_name().to_string(),
            // A model id is a leaf: nothing follows it.
            more: false,
        })
        .collect();
    out.sort_by(|a, b| a.value.cmp(&b.value));
    out
}

/// The profile roster (the built-in `oh-my-pi` included).
fn profiles(cfg: &Config) -> Vec<Candidate> {
    profile::list(cfg)
        .into_iter()
        .map(|name| Candidate {
            value: name,
            detail: String::new(),
            more: false,
        })
        .collect()
}
