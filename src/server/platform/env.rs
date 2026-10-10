//! Directories, and the environment variables behind them.

use std::path::PathBuf;

use super::os::{self, Family};

/// The user's home directory. `None` = this system has no answer yet, or the
/// variable is unset.
pub fn home() -> Option<PathBuf> {
    match os::current().family {
        Family::Unix => var_os("HOME").map(PathBuf::from),
        Family::Windows => None,
    }
}

/// Persistent, machine-generated data: the sessions database, the browser
/// profile. `$XDG_DATA_HOME/mypi`, else `~/.local/share/mypi`.
pub fn data_dir() -> PathBuf {
    base("XDG_DATA_HOME", ".local/share").join("mypi")
}

/// Configuration the user edits: `$XDG_CONFIG_HOME/mypi`, else `~/.config/mypi`. A missing
/// `config.yaml` is written here.
pub fn config_dir() -> PathBuf {
    base("XDG_CONFIG_HOME", ".config").join("mypi")
}

/// Scratch space: the system's per-user temporary directory.
pub fn temp_dir() -> PathBuf {
    std::env::temp_dir()
}

/// Where a cross-process endpoint's name lives: `$XDG_RUNTIME_DIR`, else a `mypi` directory
/// under [`temp_dir`] (created 0700 on unix).
pub fn runtime_dir() -> PathBuf {
    let dir = match (os::current().family, var_os("XDG_RUNTIME_DIR")) {
        (Family::Unix, Some(dir)) => PathBuf::from(dir),
        _ => temp_dir().join("mypi"),
    };
    let _ = std::fs::create_dir_all(&dir);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
    }
    dir
}

/// One environment variable, as UTF-8. `None` = unset or not text.
pub fn var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

/// One environment variable, uninterpreted. Paths come through here: a
/// directory name need not be valid UTF-8. Empty counts as unset.
pub fn var_os(name: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(name).filter(|v| !v.is_empty())
}

/// An XDG base directory: the variable if it is set, else the specification's
/// default under the home directory.
fn base(variable: &str, fallback: &str) -> PathBuf {
    if os::current().family != Family::Unix {
        return PathBuf::from(".");
    }
    if let Some(dir) = var_os(variable) {
        return PathBuf::from(dir);
    }
    match home() {
        Some(home) => home.join(fallback),
        None => PathBuf::from("."),
    }
}
