//! Tools. The root holds the four shared pieces: `spec`, `registry`, `executor`, `workspace`; every
//! tool gets its own folder (rationale in `src/server/agent/README.md`).

mod bash;
mod cd;
mod context;
mod edit;
mod executor;
mod mass_edit;
mod read;
mod registry;
mod spec;
mod todo;
mod workspace;
mod write;

#[cfg(feature = "web")]
mod browser;
#[cfg(feature = "web")]
mod fetch;
#[cfg(feature = "web")]
mod search;

pub use context::Window;
pub use executor::execute;
pub use registry::{TOOLS, roster};
pub use spec::{
    Args, Artifacts, BashEnv, Env, Inbox, Kind, Param, Run, Spill, Tool, ToolOutput, ToolProgress,
};
pub use todo::state::TodoPhase;
pub use workspace::Workspace;
