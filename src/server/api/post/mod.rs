//! Post — one change of session state, with a receipt.

use anyhow::Result;

use crate::ipc::contract::Viewer;
use crate::server::api::driver::{self, Driver};
use crate::server::config::Config;
use crate::server::storage::{SessionChange, Submit};

pub mod init;
pub mod server;
pub mod session;
pub mod tool;

pub use crate::ipc::contract::{ModelId, Post, PostDone};

pub fn apply(cfg: &Config, viewer: &Viewer, post: Post) -> Result<PostDone> {
    let state = driver::state(cfg)?;
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    dispatch(&mut state, viewer, post)
}

fn dispatch(driver: &mut Driver, viewer: &Viewer, post: Post) -> Result<PostDone> {
    match post {
        Post::Session { session, write } => session::apply(driver, viewer, session, write),
        Post::Tool(write) => tool::apply(driver, write),
        Post::Server(write) => server::apply(driver, write),
        Post::Init {
            cwd,
            model,
            name,
            profile,
        } => {
            if let Some(profile) = &profile {
                crate::commands::Profile::resolve(&driver.cfg, profile)?;
            }
            let session = init::create(
                &driver.cfg,
                &driver.writer,
                &cwd,
                model.as_ref(),
                name.as_deref(),
            )?;
            if let Some(profile) = profile {
                driver.writer.push(Submit::Session {
                    session,
                    change: SessionChange::Profile(profile.clone()),
                })?;
                driver
                    .sessions
                    .choose_profile(&driver.cfg, session, &profile)?;
            }
            Ok(PostDone::Created(session))
        }
    }
}
