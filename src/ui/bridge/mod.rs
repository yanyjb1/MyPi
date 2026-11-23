// The bridge: one HTTP door onto the server, for every front end that is not this terminal.

mod events;
mod http;
mod hub;
mod manifest;
mod route;
mod rpc;
mod run;
mod web;

pub use manifest::{FILE, Manifest};
pub use run::{Options, run};
pub use web::Webui;
