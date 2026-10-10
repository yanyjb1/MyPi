//! Cross-process plumbing: a unix socket, or a named pipe on Windows (not
//! written yet — see the `unimplemented!` bodies).

use std::io;
use std::path::Path;

use super::os::{self, Family};

/// Which kind of endpoint to use. `config.server.platform.ipc` decides, and
/// `os` means "whatever this system does".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cost {
    /// A unix socket: a file in the runtime directory.
    Unix,
    /// A Windows named pipe.
    Pipe,
}

impl Cost {
    /// The endpoint this system uses when the config says `os` (the default).
    pub fn of_this_system() -> Cost {
        match os::current().family {
            Family::Unix => Cost::Unix,
            Family::Windows => Cost::Pipe,
        }
    }
}

#[cfg(unix)]
mod imp {
    use std::io;
    use std::path::{Path, PathBuf};

    /// The listening end: waits for front ends, one at a time.
    pub struct Listener {
        inner: std::os::unix::net::UnixListener,
        path: PathBuf,
    }

    /// One connected end. Cloning gives two threads their own half of the same
    /// socket; duplicating a descriptor can fail, hence the `expect`.
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
        // A file that is already gone is the outcome we wanted.
        let _ = std::fs::remove_file(path);
    }

    impl Listener {
        /// Wait for the next front end.
        pub fn accept(&self) -> io::Result<Stream> {
            self.inner.accept().map(|(s, _)| Stream(s))
        }

        /// Where this listener lives, so a server can take it away on exit.
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
}

#[cfg(windows)]
mod imp {
    use std::io;
    use std::path::Path;

    const NOT_YET: &str = "named pipes are not written yet — see src/ipc/README.md";

    /// The listening end of a named pipe.
    #[allow(dead_code)]
    pub struct Listener;

    /// One connected end of a named pipe.
    pub struct Stream;

    pub fn bind(_path: &Path) -> io::Result<Listener> {
        unimplemented!("{NOT_YET}")
    }

    pub fn connect(_path: &Path) -> io::Result<Stream> {
        unimplemented!("{NOT_YET}")
    }

    /// A pipe name is not a file, so there is nothing to remove.
    pub fn unlink(_path: &Path) {}

    impl Listener {
        pub fn accept(&self) -> io::Result<Stream> {
            unimplemented!("{NOT_YET}")
        }

        pub fn local_path(&self) -> &Path {
            unimplemented!("{NOT_YET}")
        }
    }

    impl io::Read for Stream {
        fn read(&mut self, _out: &mut [u8]) -> io::Result<usize> {
            unimplemented!("{NOT_YET}")
        }
    }

    impl io::Write for Stream {
        fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
            unimplemented!("{NOT_YET}")
        }

        fn flush(&mut self) -> io::Result<()> {
            unimplemented!("{NOT_YET}")
        }
    }
}

pub use imp::{Listener, Stream};

/// Bind the endpoint at `path`; a leftover there is `unlink`'s business.
pub fn bind(cost: Cost, path: &Path) -> io::Result<Listener> {
    match cost {
        #[cfg(unix)]
        Cost::Unix => imp::bind(path),
        #[cfg(windows)]
        Cost::Pipe => imp::bind(path),
        #[allow(unreachable_patterns)]
        other => unimplemented!("{other:?} is not written yet — see src/ipc/README.md"),
    }
}

/// Connect to the endpoint at `path`.
pub fn connect(cost: Cost, path: &Path) -> io::Result<Stream> {
    match cost {
        #[cfg(unix)]
        Cost::Unix => imp::connect(path),
        #[cfg(windows)]
        Cost::Pipe => imp::connect(path),
        #[allow(unreachable_patterns)]
        other => unimplemented!("{other:?} is not written yet — see src/ipc/README.md"),
    }
}

/// Remove what `bind` left behind. A no-op where the endpoint is not a file.
pub fn unlink(cost: Cost, path: &Path) {
    match cost {
        #[cfg(unix)]
        Cost::Unix => imp::unlink(path),
        #[cfg(windows)]
        Cost::Pipe => imp::unlink(path),
        #[allow(unreachable_patterns)]
        _ => {}
    }
}
