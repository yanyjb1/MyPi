//! Windows named pipes: interface reserved, implementation still absent.

use std::io;
use std::path::Path;

pub struct Listener;

pub struct Stream;

impl Clone for Stream {
    fn clone(&self) -> Stream {
        Stream
    }
}

pub fn bind(_name: &str) -> io::Result<Listener> {
    Err(unsupported())
}

pub fn connect(_name: &str) -> io::Result<Stream> {
    Err(unsupported())
}

pub fn unlink(_name: &str) {}

impl Listener {
    pub fn accept(&self) -> io::Result<Stream> {
        Err(unsupported())
    }

    pub fn local_path(&self) -> &Path {
        Path::new("")
    }
}

impl io::Read for Stream {
    fn read(&mut self, _out: &mut [u8]) -> io::Result<usize> {
        Err(unsupported())
    }
}

impl io::Write for Stream {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(unsupported())
    }

    fn flush(&mut self) -> io::Result<()> {
        Err(unsupported())
    }
}

fn unsupported() -> io::Error {
    io::Error::new(
        io::ErrorKind::Unsupported,
        "Windows named pipes are not written yet",
    )
}
