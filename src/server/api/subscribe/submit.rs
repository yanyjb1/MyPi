//! Submit: open a round and put the connection on its turn.

use anyhow::Result;
use std::sync::mpsc;

use crate::ipc::contract::{Frame, Viewer};
use crate::server::api::driver::Driver;

pub(crate) fn apply(
    driver: &mut Driver,
    viewer: &Viewer,
    session: i64,
    prefix: Option<&str>,
    text: &str,
    suffix: Option<&str>,
) -> Result<(mpsc::Receiver<Frame>, Option<i64>)> {
    let (tx, rx) = mpsc::channel();
    let turn = driver.submit(viewer, session, prefix, text, suffix, tx)?;
    Ok((rx, turn))
}
