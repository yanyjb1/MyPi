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

use crate::server::config::BrowserConfig;
use crate::web::WebArgs;

pub(crate) use page::RENDER_WAIT;
pub use session::close_session;
pub(crate) use session::{
    clear_challenged, ensure_headed, has_display, is_challenged, mark_challenged, with_search,
};

pub(crate) fn read(args: &WebArgs, cfg: &BrowserConfig, session: i64) -> Result<String> {
    read::run(args, cfg, session)
}

pub(crate) fn open(args: &WebArgs, cfg: &BrowserConfig, session: i64) -> Result<String> {
    open::run(args, cfg, session)
}

pub(crate) fn actions(args: &WebArgs, cfg: &BrowserConfig, session: i64) -> Result<String> {
    actions::run(args, cfg, session)
}

pub(crate) fn net(args: &WebArgs, cfg: &BrowserConfig, session: i64) -> Result<String> {
    net::run(args, cfg, session)
}

pub(crate) fn screenshot(args: &WebArgs, cfg: &BrowserConfig, session: i64) -> Result<String> {
    screenshot::run(args, cfg, session)
}
