//! The web tool's operations: one file per verb family, each over a leased tab.

use std::time::Duration;

/// Ceiling for one evaluated action, and for the resource scan beside it.
pub(crate) const ACTION_TIMEOUT: Duration = Duration::from_secs(10);

pub(crate) mod drive;
pub(crate) mod open;
pub(crate) mod read;
pub(crate) mod screenshot;
