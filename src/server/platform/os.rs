//! Which system this is.

/// The one distinction that changes behavior today.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// Linux, macOS, the BSDs.
    Unix,
    /// Windows.
    Windows,
}

/// This machine, as far as the build knows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Os {
    pub family: Family,
    /// `std::env::consts::OS`: `"linux"`, `"macos"`, `"windows"`, ….
    pub name: &'static str,
    /// `std::env::consts::ARCH`: `"x86_64"`, `"aarch64"`, ….
    pub arch: &'static str,
}

/// The system this binary was built for.
pub fn current() -> Os {
    Os {
        family: if cfg!(windows) {
            Family::Windows
        } else {
            Family::Unix
        },
        name: std::env::consts::OS,
        arch: std::env::consts::ARCH,
    }
}
