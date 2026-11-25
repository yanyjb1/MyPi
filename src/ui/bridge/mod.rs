// The bridge: one HTTP door onto the server, for every front end that is not this terminal.

mod adapter;
mod http;
mod manager;
mod run;

pub use adapter::manifest::{FILE, Manifest};
pub use adapter::web::Webui;
pub use run::{Options, run};
