// The browser domain: what a call may do, and the browser it does it through.

mod args;
mod artifact;
mod devtools;
mod halt;
mod html;
mod lifecycle;
mod ops;
mod page;
mod process;
mod run;
mod search;
mod seen;
mod tabs;
mod url;
mod verify;

pub use args::WebArgs;
pub use lifecycle::shutdown;
pub use run::run;
pub(crate) use search::ENGINES;
pub use tabs::close_session as closed_tabs;

// A session's end also takes what its reads had shown the model.
pub fn close_session(session: i64) {
    closed_tabs(session);
    seen::forget(session);
}
