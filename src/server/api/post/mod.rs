//! Post — the thirteen writes, one function per name.

use anyhow::Result;
use serde::Deserialize;

use crate::ipc::contract::Viewer;
use crate::ipc::contract::post::{self, ask, reply};
use crate::server::api::driver::{self, Driver};
use crate::server::config::Config;
use crate::server::storage::{SessionChange, Submit};

pub mod init;
pub mod server;
pub mod session;
pub mod tool;

pub fn init(cfg: &Config, _viewer: &Viewer, ask: ask::Init) -> Result<serde_json::Value> {
    let state = driver::state(cfg)?;
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    let session = init::create(
        &state.cfg,
        &state.writer,
        &ask.cwd,
        ask.model.as_ref(),
        ask.name.as_deref(),
    )?;
    if let Some(profile) = &ask.profile {
        crate::commands::Profile::resolve(&state.cfg, profile)?;
        state.writer.push(Submit::Session {
            session,
            change: SessionChange::Profile(profile.clone()),
        })?;
        state
            .sessions
            .choose_profile(&state.cfg, session, profile)?;
    }
    Ok(serde_json::to_value(reply::Init { session })?)
}

pub fn name(cfg: &Config, viewer: &Viewer, ask: ask::Name) -> Result<serde_json::Value> {
    with_driver(cfg, |driver| {
        session::name(driver, ask.session, ask.name)?;
        Ok(serde_json::json!({}))
    })
}

pub fn cwd(cfg: &Config, viewer: &Viewer, ask: ask::Cwd) -> Result<serde_json::Value> {
    with_driver(cfg, |driver| {
        let cwd = session::cwd(driver, ask.session, ask.path)?;
        Ok(serde_json::to_value(reply::Cwd { cwd })?)
    })
}

pub fn switch(cfg: &Config, viewer: &Viewer, ask: ask::Switch) -> Result<serde_json::Value> {
    with_driver(cfg, |driver| {
        session::switch::apply(driver, ask.session, ask.model)?;
        Ok(serde_json::json!({}))
    })
}

pub fn profile(cfg: &Config, viewer: &Viewer, ask: ask::Profile) -> Result<serde_json::Value> {
    with_driver(cfg, |driver| {
        session::profile::apply(driver, ask.session, ask.profile)?;
        Ok(serde_json::json!({}))
    })
}

pub fn todo_on(cfg: &Config, viewer: &Viewer, ask: ask::Todo) -> Result<serde_json::Value> {
    with_driver(cfg, |driver| {
        session::todo::apply(driver, ask.session, true, ask.call_id)?;
        Ok(serde_json::json!({}))
    })
}

pub fn todo_off(cfg: &Config, viewer: &Viewer, ask: ask::Todo) -> Result<serde_json::Value> {
    with_driver(cfg, |driver| {
        session::todo::apply(driver, ask.session, false, ask.call_id)?;
        Ok(serde_json::json!({}))
    })
}

pub fn delete(cfg: &Config, viewer: &Viewer, ask: ask::Session) -> Result<serde_json::Value> {
    with_driver(cfg, |driver| {
        if driver.running.flag(ask.session).is_some() {
            return Ok(busy());
        }
        session::delete::apply(driver, ask.session)?;
        Ok(serde_json::json!({}))
    })
}

pub fn interrupt(cfg: &Config, viewer: &Viewer, ask: ask::Session) -> Result<serde_json::Value> {
    with_driver(cfg, |driver| {
        session::interrupt::apply(driver, ask.session);
        Ok(serde_json::json!({}))
    })
}

pub fn rewind(cfg: &Config, viewer: &Viewer, ask: ask::Rewind) -> Result<serde_json::Value> {
    with_driver(cfg, |driver| {
        if driver.running.flag(ask.session).is_some() {
            return Ok(busy());
        }
        let cut = session::rewind::apply(driver, ask.session, ask.turn, ask.block)?;
        Ok(serde_json::to_value(reply::Rewind { cut })?)
    })
}

pub fn unwatch(cfg: &Config, viewer: &Viewer, ask: ask::Session) -> Result<serde_json::Value> {
    with_driver(cfg, |driver| {
        session::unwatch::apply(driver, viewer, ask.session);
        Ok(serde_json::json!({}))
    })
}

pub fn default_model(
    cfg: &Config,
    viewer: &Viewer,
    ask: ask::DefaultModel,
) -> Result<serde_json::Value> {
    with_driver(cfg, |driver| {
        server::default_model::apply(driver, ask.model)?;
        Ok(serde_json::json!({}))
    })
}

pub fn reload(cfg: &Config, viewer: &Viewer) -> Result<serde_json::Value> {
    with_driver(cfg, |driver| {
        server::reload::apply(driver)?;
        Ok(serde_json::json!({}))
    })
}

fn busy() -> serde_json::Value {
    crate::ipc::contract::error::payload(
        crate::ipc::contract::ErrorCode::Busy,
        "the session is busy",
    )
}

fn with_driver<T>(cfg: &Config, f: impl FnOnce(&mut Driver) -> Result<T>) -> Result<T> {
    let state = driver::state(cfg)?;
    let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut state)
}
