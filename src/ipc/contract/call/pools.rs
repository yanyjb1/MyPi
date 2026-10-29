//! `call.pools`: the two completion pools.

use crate::commands::Candidate;

/// The completion pools: model ids and profile names.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct CandidatePools {
    pub models: Vec<Candidate>,
    pub profiles: Vec<Candidate>,
}

/// Request: `{}`.
#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize)]
pub struct Ask {}

/// Reply: `{models, profiles}`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Reply {
    pub models: Vec<Candidate>,
    pub profiles: Vec<Candidate>,
}
