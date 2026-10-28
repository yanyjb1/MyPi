//! Cross-process connection mechanics, one implementation per platform family.

use std::io;

use crate::ipc::endpoint::Endpoint;

pub use crate::ipc::endpoint::Kind;

#[cfg(unix)]
pub mod unix;
#[cfg(windows)]
pub mod windows;

#[cfg(unix)]
pub use unix::{Listener, Stream};
#[cfg(windows)]
pub use windows::{Listener, Stream};

pub fn bind(endpoint: &Endpoint) -> io::Result<Listener> {
    match endpoint {
        #[cfg(unix)]
        Endpoint::Unix { path } => unix::bind(path),
        #[cfg(windows)]
        Endpoint::Pipe { name } => windows::bind(name),
        #[allow(unreachable_patterns)]
        other => unsupported(other),
    }
}

pub fn connect(endpoint: &Endpoint) -> io::Result<Stream> {
    match endpoint {
        #[cfg(unix)]
        Endpoint::Unix { path } => unix::connect(path),
        #[cfg(windows)]
        Endpoint::Pipe { name } => windows::connect(name),
        #[allow(unreachable_patterns)]
        other => unsupported(other),
    }
}

pub fn unlink(endpoint: &Endpoint) {
    match endpoint {
        #[cfg(unix)]
        Endpoint::Unix { path } => unix::unlink(path),
        #[cfg(windows)]
        Endpoint::Pipe { .. } => {}
        #[allow(unreachable_patterns)]
        _ => {}
    }
}

fn unsupported<T>(endpoint: &Endpoint) -> io::Result<T> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        format!("{endpoint:?} is not written on this platform"),
    ))
}
