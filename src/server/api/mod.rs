//! api — the front-end surface: one router on `(kind, name)`, and the session streams.

pub(crate) mod call;
mod convert;
mod driver;
pub(crate) mod post;
pub(crate) mod register;
mod route;
pub mod subscribe;

pub use crate::ipc::contract::{Frame, Kind};
use std::sync::mpsc::Receiver;

use crate::ipc::contract::Viewer;
use crate::server::config::Config;

/// What one inbound frame turns into.
pub enum Response {
    /// Write this frame back on the same connection.
    Frame(Frame),
    /// Hand the connection to the stream pump.
    Stream(Receiver<Frame>),
    /// Nothing goes back.
    Silence,
}

/// The backend's one surface for the IPC server: it owns the config, not the transport.
pub struct Api {
    cfg: Config,
}

impl Api {
    pub fn new(cfg: Config) -> Api {
        Api { cfg }
    }

    /// Route one inbound frame. The only place that reads `(kind, name)`.
    pub fn receive(&self, viewer: &Viewer, frame: Frame) -> Response {
        match frame.kind {
            Kind::Register => register::receive(viewer, &frame),
            Kind::Call => route::call(&self.cfg, viewer, frame),
            Kind::Post => route::post(&self.cfg, viewer, frame),
            Kind::Subscribe => route::subscribe(&self.cfg, viewer, frame),
            Kind::Notice => unknown_name(frame),
        }
    }
}

/// `notice` has no inbound name.
fn unknown_name(frame: Frame) -> Response {
    Response::Frame(Frame::error_to(
        &frame,
        crate::ipc::contract::ErrorCode::UnknownName,
        "this name has no inbound form",
    ))
}
