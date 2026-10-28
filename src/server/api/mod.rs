//! api — the front-end surface: three doors (`call`, `post`, `subscribe`), and the server's own
//! notice.

pub mod call;
mod convert;
mod driver;
pub mod notice;
pub mod post;
pub mod subscribe;

pub use call::{Call, Reply};
pub use notice::{Notice, NoticeLevel, notify};
pub use post::{Post, PostDone};
pub use subscribe::{Event, Opened, Subscribe};

use std::sync::mpsc::Receiver;

use crate::ipc::contract::{Retention, Viewer};
use crate::server::config::Config;

/// The backend's one surface for the IPC server: it owns the config, not the transport.
pub struct Api {
    cfg: Config,
}

impl Api {
    pub fn new(cfg: Config) -> Api {
        Api { cfg }
    }

    pub fn call(&self, viewer: &Viewer, call: Call) -> anyhow::Result<Reply> {
        call::answer(&self.cfg, viewer, call)
    }

    pub fn post(&self, viewer: &Viewer, post: Post) -> anyhow::Result<PostDone> {
        post::apply(&self.cfg, viewer, post)
    }

    pub fn connect(&self, viewer: &Viewer) -> anyhow::Result<Receiver<Event>> {
        subscribe::connect(&self.cfg, viewer)
    }

    pub fn watch(&self, viewer: &Viewer, session: i64) -> anyhow::Result<()> {
        subscribe::watch(&self.cfg, viewer, session)
    }

    pub fn submit(
        &self,
        viewer: &Viewer,
        session: i64,
        prefix: Option<&str>,
        text: &str,
        suffix: Option<&str>,
    ) -> anyhow::Result<Opened> {
        subscribe::submit(&self.cfg, viewer, session, prefix, text, suffix)
    }

    pub fn compact(
        &self,
        viewer: &Viewer,
        session: i64,
        retention: Retention,
        focus: Option<String>,
    ) -> anyhow::Result<Opened> {
        subscribe::compact(&self.cfg, viewer, session, retention, focus)
    }
}
