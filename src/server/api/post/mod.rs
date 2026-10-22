//! Post — change session state. [`apply`] is the door; some changes are the driver's own to finish
//! (a profile landing mid-session, a switched connection, an unwatch).

use anyhow::Result;
use std::path::PathBuf;

use crate::server::api::driver::{self, Driver};
use crate::server::config::Config;
use crate::server::storage::{Reader, Target};

use super::types::{Post, PostDone};

pub mod cwd;
pub mod delete;
pub mod init;
pub mod model;
pub mod name;
pub mod profile;
pub mod rewind;

/// Carry out one change.
pub fn apply(cfg: &Config, post: Post) -> Result<PostDone> {
    let state = driver::state(cfg)?;
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    dispatch(&mut state, post)
}

/// The dispatch, on the driver's own state.
fn dispatch(state: &mut Driver, post: Post) -> Result<PostDone> {
    // What the driver's own state has to remember, read before the change
    // consumes it.
    let (profile_session, profile_name) = match &post {
        Post::Profile { session, profile } => (Some(*session), Some(profile.clone())),
        Post::Init { profile, .. } => (None, profile.clone()),
        _ => (None, None),
    };
    let switched = match &post {
        Post::Switch { session, id } => Some((*session, id.addressed())),
        _ => None,
    };
    let removed = match &post {
        Post::Delete { session } => Some(*session),
        _ => None,
    };

    let done = match post {
        // Watching is the driver's own state: no row, no other door.
        Post::Unwatch { viewer, session } => {
            state.unwatch(viewer, session);
            PostDone::Done
        }
        Post::Interrupt { session } => {
            state.interrupt(session);
            PostDone::Done
        }
        Post::Rewind {
            session,
            turn,
            block,
        } => {
            if state.running.flag(session).is_some() {
                return Ok(PostDone::Busy);
            }
            state.rewind(Target {
                session,
                turn,
                block,
            })?;
            PostDone::Done
        }
        Post::Name { session, name } => {
            crate::commands::Name::set(&state.writer, session, name.as_deref())?;
            PostDone::Done
        }
        Post::Cwd { session, path } => {
            let cwd = live_cwd(state, session)?;
            let conn = Reader::open(&state.db)?;
            let out = crate::commands::Cdp::set(&state.writer, &conn, session, &cwd, &path);
            Reader::close(conn);
            out?;
            PostDone::Done
        }
        Post::Switch { session, id } => {
            crate::commands::Switch::set(&state.cfg, &state.writer, session, &id)?;
            PostDone::Done
        }
        Post::DefaultModel { id } => {
            crate::commands::Model::set_default(&state.cfg, &id)?;
            PostDone::Done
        }
        Post::Profile { profile, .. } => {
            crate::commands::Profile::resolve(&state.cfg, &profile)?;
            PostDone::Done
        }
        Post::Init {
            cwd,
            model,
            name,
            profile,
        } => {
            // A profile that is not on the roster is an error here, not a
            // fallback: the driver is about to run this session under it.
            if let Some(profile) = &profile {
                crate::commands::Profile::resolve(&state.cfg, profile)?;
            }
            let session = init::create(
                &state.cfg,
                &state.writer,
                &cwd,
                model.as_ref(),
                name.as_deref(),
            )?;
            PostDone::Created(session)
        }
        Post::Delete { session } => {
            if state.running.flag(session).is_some() {
                return Ok(PostDone::Busy);
            }
            crate::commands::Delete::remove(&state.writer, session)?;
            PostDone::Done
        }
    };

    let session = match done {
        PostDone::Created(session) => Some(session),
        PostDone::Done => profile_session,
        PostDone::Busy => None,
    };
    if let (Some(session), Some(name)) = (session, profile_name) {
        state.sessions.choose_profile(&state.cfg, session, &name)?;
    }
    if let Some((session, id)) = switched {
        state.sessions.retarget(&state.cfg, session, &id)?;
    }
    if let Some(session) = removed {
        state.sessions.forget(session);
    }
    Ok(done)
}

/// The session's working directory as it stands in memory, for a `/cdp` that
/// has to resolve a relative path against it.
fn live_cwd(state: &Driver, session: i64) -> Result<PathBuf> {
    let conn = Reader::open(&state.db)?;
    let cwd = state.sessions.cwd(&state.cfg, &conn, session);
    Reader::close(conn);
    cwd
}
