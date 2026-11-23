// Which adapter a request speaks for, and whether it is allowed in at all.

use std::net::SocketAddr;

use super::manifest::Adapter;

// The header an adapter names itself in; it is the only place a request says who it is.
pub const HEADER: &str = "x-bridge-adapter";

// An adapter that said it is local serves this machine only, whatever the port was bound to.
pub fn admitted(adapter: &Adapter, peer: SocketAddr) -> Result<(), &'static str> {
    if adapter.local_only && !peer.ip().is_loopback() {
        return Err("this adapter serves this machine only");
    }
    Ok(())
}
