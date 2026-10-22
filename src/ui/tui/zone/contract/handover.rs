//! Screen ownership and the one transaction that changes it.

use super::request::Request;

/// Zone identity. At most one zone shows at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoneId {
    /// The chat screen: history, input, reserved.
    Main,
    /// The session picker.
    Resume,
    /// The block tree.
    Tree,
}

/// The showing zone gives the screen up, carrying its pending requests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handover {
    pub to: Option<ZoneId>,
    pub requests: Vec<Request>,
}
