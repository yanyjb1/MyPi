//! Where one database's server listens, and how a front end finds it.

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// Which connection mechanism this platform uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Unix,
    Pipe,
}

impl Kind {
    pub fn of_this_system() -> Kind {
        if cfg!(windows) {
            Kind::Pipe
        } else {
            Kind::Unix
        }
    }
}

/// One database's endpoint.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Endpoint {
    Unix { path: PathBuf },
    Pipe { name: String },
}

impl Endpoint {
    /// The endpoint this database would use on this system.
    pub fn for_db(db: &Path, kind: Kind) -> Endpoint {
        let id = fingerprint(db);
        match kind {
            Kind::Unix => Endpoint::Unix {
                path: runtime_dir().join(format!("mypi-{id}.sock")),
            },
            Kind::Pipe => Endpoint::Pipe {
                name: format!(r"\\.\pipe\mypi-{id}"),
            },
        }
    }

    /// The file that tells a front end where this database's server is.
    pub fn file_path(db: &Path) -> PathBuf {
        runtime_dir().join(format!("mypi-{}.endpoint", fingerprint(db)))
    }
}

/// Read the endpoint file.
pub fn read(file: &Path) -> std::io::Result<Endpoint> {
    let text = std::fs::read_to_string(file)?;
    serde_json::from_str(&text).map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

/// Write the endpoint file atomically, user-only where the platform has modes.
pub fn write(file: &Path, endpoint: &Endpoint) -> std::io::Result<()> {
    let text = serde_json::to_string(endpoint)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("endpoint.tmp");
    std::fs::write(&tmp, text)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
    }
    std::fs::rename(&tmp, file)
}

/// Remove the endpoint file. A missing file is the outcome we wanted.
pub fn remove(file: &Path) {
    let _ = std::fs::remove_file(file);
}

/// A short, stable name for one database path.
pub fn fingerprint(db: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    #[cfg(windows)]
    db.to_string_lossy().to_lowercase().hash(&mut hasher);
    #[cfg(not(windows))]
    db.hash(&mut hasher);
    format!("{:012x}", hasher.finish())
}

/// Where endpoint files live on this platform.
pub fn runtime_dir() -> PathBuf {
    let dir = platform_dir();
    let _ = std::fs::create_dir_all(&dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    dir
}

#[cfg(target_os = "linux")]
fn platform_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("mypi"))
}

#[cfg(target_os = "macos")]
fn platform_dir() -> PathBuf {
    std::env::temp_dir().join("mypi")
}

#[cfg(windows)]
fn platform_dir() -> PathBuf {
    std::env::var_os("LOCALAPPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir)
        .join("mypi")
}

#[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
fn platform_dir() -> PathBuf {
    std::env::temp_dir().join("mypi")
}
