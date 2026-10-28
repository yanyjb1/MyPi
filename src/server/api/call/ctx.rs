//! What a query group gets: the config, a read connection, the viewer's attached session, and the
//! driver's live state.

use rusqlite::Connection;

use crate::server::api::driver::running::Running;
use crate::server::api::driver::sessions::Sessions;
use crate::server::config::Config;

pub(crate) struct Ctx<'a> {
    pub(crate) cfg: &'a Config,
    pub(crate) reader: &'a Connection,
    pub(crate) attached: Option<i64>,
    pub(crate) sessions: &'a Sessions,
    pub(crate) running: &'a Running,
}
