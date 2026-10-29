//! Watch: put the connection on the session's whole stream.

use anyhow::Result;
use std::sync::mpsc;

use crate::ipc::contract::{Frame, Viewer};
use crate::server::api::driver::{Driver, subscriptions::Role};

pub(crate) fn apply(
    driver: &Driver,
    viewer: &Viewer,
    session: i64,
) -> Result<mpsc::Receiver<Frame>> {
    let (tx, rx) = mpsc::channel();
    driver
        .relay
        .attach(viewer, session, Role::Watcher, None, tx);
    Ok(rx)
}
