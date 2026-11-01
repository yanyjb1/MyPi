//! The subagent tools: one spawns a child, one lets that child finish.

mod answer;
mod r#final;
mod task;
mod template;

pub(crate) use r#final::TOOL as FINAL;
pub(crate) use task::TOOL as TASK;
