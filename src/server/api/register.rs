//! Register: hello and bye. No state, no reply.

use crate::ipc::contract::Viewer;
use crate::server::api::Response;

pub(crate) fn receive(_viewer: &Viewer, _frame: &crate::ipc::contract::Frame) -> Response {
    Response::Silence
}
