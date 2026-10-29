//! The completion pools: model ids and profile names, asked for once at startup.

use crate::commands::Candidate;
use crate::server::config::Config;
use crate::server::config::read::profile;

use crate::ipc::contract::call::pools::CandidatePools;

pub(crate) fn pools(cfg: &Config) -> CandidatePools {
    CandidatePools {
        models: models(cfg),
        profiles: profiles(cfg),
    }
}

fn models(cfg: &Config) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = cfg
        .models()
        .map(|(provider, entry)| Candidate {
            value: format!("{}:{}", provider, entry.id),
            detail: entry.display_name().to_string(),
            more: false,
        })
        .collect();
    out.sort_by(|a, b| a.value.cmp(&b.value));
    out
}

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
