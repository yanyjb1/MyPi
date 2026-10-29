//! Profile discovery and resolution. A profile is a directory under `profiles/`.

mod discover;
mod resolve;
mod types;

pub use discover::{list, locate};
pub use resolve::{compact_instruction, omp_system, resolve};
pub use types::{DEFAULT_PROFILE, OMP_PROFILE, ResolvedProfile};
