//! Session writes: metadata, the destructive cuts, and the viewer's own watch.

use anyhow::Result;

use crate::ipc::contract::Viewer;
use crate::server::api::driver::Driver;
use crate::server::api::post::{ModelId, PostDone};

pub(crate) mod cwd;
pub(crate) mod delete;
pub(crate) mod interrupt;
pub(crate) mod name;
pub(crate) mod profile;
pub(crate) mod rewind;
pub(crate) mod switch;
pub(crate) mod todo;
pub(crate) mod unwatch;

pub use crate::ipc::contract::SessionWrite;

pub(crate) fn apply(
    driver: &mut Driver,
    viewer: &Viewer,
    session: i64,
    write: SessionWrite,
) -> Result<PostDone> {
    match write {
        SessionWrite::Name(name) => {
            name::apply(driver, session, name)?;
            Ok(PostDone::Done)
        }
        SessionWrite::Cwd(path) => {
            cwd::apply(driver, session, path)?;
            Ok(PostDone::Done)
        }
        SessionWrite::Switch(id) => {
            switch::apply(driver, session, id)?;
            Ok(PostDone::Done)
        }
        SessionWrite::Profile(name) => {
            profile::apply(driver, session, name)?;
            Ok(PostDone::Done)
        }
        SessionWrite::TodoOn { call_id } => {
            todo::apply(driver, session, true, call_id)?;
            Ok(PostDone::Done)
        }
        SessionWrite::TodoOff { call_id } => {
            todo::apply(driver, session, false, call_id)?;
            Ok(PostDone::Done)
        }
        SessionWrite::Delete => {
            if driver.running.flag(session).is_some() {
                return Ok(PostDone::Busy);
            }
            delete::apply(driver, session)?;
            Ok(PostDone::Done)
        }
        SessionWrite::Interrupt => {
            interrupt::apply(driver, session);
            Ok(PostDone::Done)
        }
        SessionWrite::Rewind { turn, block } => {
            if driver.running.flag(session).is_some() {
                return Ok(PostDone::Busy);
            }
            rewind::apply(driver, session, turn, block)?;
            Ok(PostDone::Done)
        }
        SessionWrite::Unwatch => {
            unwatch::apply(driver, viewer, session);
            Ok(PostDone::Done)
        }
    }
}
