//! Unix sockets: Linux and macOS.

use std::io;
use std::path::{Path, PathBuf};

/// The listening end.
pub struct Listener {
    inner: std::os::unix::net::UnixListener,
    path: PathBuf,
}

/// One connected end.
pub struct Stream(std::os::unix::net::UnixStream);

impl Clone for Stream {
    fn clone(&self) -> Stream {
        Stream(self.0.try_clone().expect("cannot duplicate the socket"))
    }
}

pub fn bind(path: &Path) -> io::Result<Listener> {
    let inner = std::os::unix::net::UnixListener::bind(path)?;
    Ok(Listener {
        inner,
        path: path.to_path_buf(),
    })
}

pub fn connect(path: &Path) -> io::Result<Stream> {
    std::os::unix::net::UnixStream::connect(path).map(Stream)
}

pub fn unlink(path: &Path) {
    let _ = std::fs::remove_file(path);
}

impl Listener {
    pub fn accept(&self) -> io::Result<Stream> {
        self.inner.accept().map(|(s, _)| Stream(s))
    }

    pub fn local_path(&self) -> &Path {
        &self.path
    }
}

impl io::Read for Stream {
    fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
        io::Read::read(&mut self.0, out)
    }
}

impl io::Write for Stream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        io::Write::write(&mut self.0, buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        io::Write::flush(&mut self.0)
    }
}
