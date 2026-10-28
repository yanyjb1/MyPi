//! Unwatch: drop this viewer's stream and let the session's live state go when idle.

use crate::ipc::contract::Viewer;
use crate::server::api::driver::Driver;

pub(crate) fn apply(driver: &Driver, viewer: &Viewer, session: i64) {
    driver.relay.detach(viewer);
    driver.reap(session);
}
