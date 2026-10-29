//! Call — the ten read-only questions, one function per name.

use anyhow::Result;
use std::path::PathBuf;

use crate::ipc::contract::Viewer;
use crate::ipc::contract::call;
use crate::server::api::driver;
use crate::server::config::Config;
use crate::server::storage::Reader;

pub(crate) mod ctx;
pub mod server;
pub mod session;
pub mod tool;

use ctx::Ctx;

pub fn init(
    cfg: &Config,
    viewer: &Viewer,
    session: i64,
    limit: usize,
) -> Result<call::init::Reply> {
    with_ctx(cfg, viewer, |ctx| {
        session::init::answer(ctx, session, limit)
    })
}

pub fn blocks(
    cfg: &Config,
    viewer: &Viewer,
    session: i64,
    ranges: Vec<[i64; 2]>,
) -> Result<call::blocks::Reply> {
    with_ctx(cfg, viewer, |ctx| {
        Ok(call::blocks::Reply {
            blocks: session::blocks::answer(ctx, session, ranges)?,
        })
    })
}

pub fn meta(cfg: &Config, viewer: &Viewer, session: i64) -> Result<call::meta::Reply> {
    with_ctx(cfg, viewer, |ctx| session::meta::answer(ctx, session))
}

pub fn usage(
    cfg: &Config,
    viewer: &Viewer,
    session: i64,
    lang: call::usage::UsageLang,
) -> Result<call::usage::Reply> {
    with_ctx(cfg, viewer, |ctx| {
        session::usage::answer(ctx, session, lang)
    })
}

pub fn todo(cfg: &Config, viewer: &Viewer, session: i64) -> Result<call::todo::Reply> {
    with_ctx(cfg, viewer, |ctx| {
        Ok(call::todo::Reply {
            phases: tool::todo::list(ctx.reader, session)?
                .into_iter()
                .map(crate::server::api::convert::todo_phase)
                .collect(),
        })
    })
}

pub fn memo(
    cfg: &Config,
    viewer: &Viewer,
    session: Option<i64>,
    op: call::memo::MemoOp,
    scope: call::memo::MemoScope,
    name: Option<String>,
) -> Result<call::memo::Reply> {
    with_ctx(cfg, viewer, |ctx| {
        Ok(call::memo::Reply {
            data: tool::memo::answer(ctx, session, op, scope, name)?,
        })
    })
}

pub fn children(cfg: &Config, viewer: &Viewer, session: i64) -> Result<call::children::Reply> {
    with_ctx(cfg, viewer, |ctx| {
        Ok(call::children::Reply {
            rows: tool::children::list(ctx, session)?,
        })
    })
}

pub fn config(cfg: &Config, viewer: &Viewer, key: &str) -> Result<call::config::Reply> {
    with_ctx(cfg, viewer, |ctx| {
        Ok(call::config::Reply {
            value: server::config::value(ctx, key)?,
        })
    })
}

pub fn pools(cfg: &Config, viewer: &Viewer) -> Result<call::pools::Reply> {
    with_ctx(cfg, viewer, |ctx| {
        let pools = server::pools::pools(ctx.cfg);
        Ok(call::pools::Reply {
            models: pools.models,
            profiles: pools.profiles,
        })
    })
}

pub fn sessions(
    cfg: &Config,
    viewer: &Viewer,
    under: Option<PathBuf>,
) -> Result<call::sessions::Reply> {
    with_ctx(cfg, viewer, |ctx| {
        Ok(call::sessions::Reply {
            rows: server::sessions::rows(ctx.reader, under.as_deref())?,
        })
    })
}

fn with_ctx<T>(cfg: &Config, viewer: &Viewer, f: impl FnOnce(&Ctx) -> Result<T>) -> Result<T> {
    let state = driver::state(cfg)?;
    let state = state.lock().unwrap_or_else(|e| e.into_inner());
    let conn = Reader::open(&state.db)?;
    let ctx = Ctx {
        cfg: &state.cfg,
        reader: &conn,
        attached: state.relay.session_of(viewer),
        sessions: &state.sessions,
        running: &state.running,
    };
    let out = f(&ctx);
    Reader::close(conn);
    out
}
