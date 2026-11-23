// Unix sockets: every unix, Android included.

use std::io;
use std::path::Path;

// The listening end.
pub struct Listener {
    inner: std::os::unix::net::UnixListener,
}

// One connected end.
pub struct Stream(std::os::unix::net::UnixStream);

impl Clone for Stream {
    fn clone(&self) -> Stream {
        Stream(self.0.try_clone().expect("cannot duplicate the socket"))
    }
}

pub fn bind(path: &Path) -> io::Result<Listener> {
    let inner = std::os::unix::net::UnixListener::bind(path)?;
    Ok(Listener { inner })
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
}

impl Stream {
    // Close both directions: a blocking read on any clone returns end of file.
    pub fn shutdown(&self) {
        let _ = self.0.shutdown(std::net::Shutdown::Both);
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
