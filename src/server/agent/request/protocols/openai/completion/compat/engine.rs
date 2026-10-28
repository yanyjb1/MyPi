//! Detection: `(base_url, model_id, user switches)` → a settled `Resolved`. Baseline → rule table →
//! user switches; adding a provider is a data change, not a code change.

use super::defaults::is_local;
use super::rules::{Probe, Rule, host_of, rules};
use super::types::*;

impl Resolved {
    /// Detect by base_url and model id: baseline, then the rule table (priority, then declaration
    /// order), then the user's switches.
    pub fn detect(base_url: &str, model_id: &str) -> Self {
        let mut r = Self::conservative();
        let host = host_of(base_url);
        let model = model_id.to_ascii_lowercase();
        let probe = Probe {
            host: &host,
            model: &model,
            is_private: is_local(base_url),
        };
        // `Config::load` validated the table at startup; failing here means the file was broken while
        // running, and the baseline beats a panic.
        if let Ok(list) = rules() {
            let mut matched: Vec<&Rule> = list.iter().filter(|rule| rule.matches(&probe)).collect();
            // The user's table sorts after the built-in one; within a source, priority order (stable).
            matched.sort_by_key(|rule| (rule.source, rule.priority));
            for rule in matched {
                r.apply(&rule.set);
            }
        }
        r
    }
}

pub fn resolve(base_url: &str, model_id: &str, user: Option<&Compat>) -> Resolved {
    let mut r = Resolved::detect(base_url, model_id);
    if let Some(u) = user {
        r.apply(u);
    }
    r
}
