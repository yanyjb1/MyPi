//! The subagent tools and the service behind them.

mod contract;
mod draft;
mod r#final;
mod runner;
mod service;
mod spawn;
mod task;
mod template;

pub(crate) use contract::START;
pub(crate) use r#final::TOOL as FINAL;
pub(crate) use service::{Subagent, SubagentFactory};
pub(crate) use spawn::{ChildSpec, Spawn};
pub(crate) use task::TOOL as TASK;
