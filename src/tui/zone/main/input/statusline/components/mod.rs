//! The standard statusline components.
//!
//! One file per component; [`standard`] is the registry. Adding a component is
//! a new file plus one line here — the engine sorts by `(side, order)`,
//! inserts the connectors and applies the omission priority, so nothing else
//! has to change.

pub mod activity;
pub mod cost;
pub mod cwd;
pub mod git;
pub mod model;
pub mod session;
pub mod usage;

use super::StatusComponent;

/// The standard row, in registration order. `StatusLine::new` sorts it by
/// `(side, order)`, so the order here only matters for ties.
pub(super) fn standard() -> Vec<Box<dyn StatusComponent>> {
    vec![
        Box::new(activity::Activity::default()),
        Box::new(model::Model::default()),
        Box::new(cwd::Cwd::default()),
        Box::new(git::Git::default()),
        Box::new(cost::Cost::default()),
        Box::new(usage::Usage::default()),
        Box::new(session::Session::default()),
    ]
}
