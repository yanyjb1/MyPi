// The web domain: what a call may do, and the browser it does it through.

mod args;
mod artifact;
mod browser;
mod halt;
mod html;
mod ops;
mod run;
mod search;
mod seen;
mod url;

pub use args::WebArgs;
pub use browser::lifecycle::shutdown;
pub use browser::tabs::close_session as closed_tabs;
pub use run::run;
pub(crate) use search::ENGINES;

// A session's end also takes what its reads had shown the model.
pub fn close_session(session: i64) {
    closed_tabs(session);
    seen::forget(session);
}
