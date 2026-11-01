mod actions;
mod cdp;
mod net;
mod open;
mod page;
mod process;
mod read;
mod reaper;
mod screenshot;
mod session;

use anyhow::Result;

use crate::web::WebArgs;

pub(crate) use page::RENDER_WAIT;
pub use session::close_session;
pub(crate) use session::{
    clear_challenged, ensure_headed, has_display, is_challenged, mark_challenged, with_search,
};

pub(crate) fn read(args: &WebArgs, session: i64) -> Result<String> {
    read::run(args, session)
}

pub(crate) fn open(args: &WebArgs, session: i64) -> Result<String> {
    open::run(args, session)
}

pub(crate) fn actions(args: &WebArgs, session: i64) -> Result<String> {
    actions::run(args, session)
}

pub(crate) fn net(args: &WebArgs, session: i64) -> Result<String> {
    net::run(args, session)
}

pub(crate) fn screenshot(args: &WebArgs, session: i64) -> Result<String> {
    screenshot::run(args, session)
}
