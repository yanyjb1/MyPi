// Directories, and the environment variables behind them.

use std::path::PathBuf;
use std::sync::{LazyLock, RwLock};

use super::os::{self, Family};

// The user's home directory. `None` = the variable is unset.
pub fn home() -> Option<PathBuf> {
    match os::current().family {
        Family::Unix => var_os("HOME").map(PathBuf::from),
        Family::Windows => var_os("USERPROFILE").map(PathBuf::from),
    }
}

// What `~` stands for: the directory the run anchored, else the platform's home.
static TILDE: LazyLock<RwLock<Option<PathBuf>>> = LazyLock::new(|| RwLock::new(None));

// Fix what `~` stands for, once, at startup. `None` keeps the platform's own home.
pub fn anchor(named: Option<PathBuf>) {
    *TILDE.write().unwrap_or_else(|e| e.into_inner()) = named;
}

// What `~` stands for right now.
pub fn tilde_home() -> Option<PathBuf> {
    TILDE
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .or_else(home)
}

// One path with a leading `~` replaced: `~`, `~/x` and `~\\x` alike. `None` = a `~` this process cannot expand: another user's home, or no home at all.
pub fn expand(raw: &str) -> Option<PathBuf> {
    let Some(rest) = raw.strip_prefix('~') else {
        return Some(PathBuf::from(raw));
    };
    if !rest.is_empty() && !rest.starts_with(['/', '\\']) {
        return None;
    }
    let rest = rest.trim_start_matches(['/', '\\']);
    let home = tilde_home()?;
    Some(match rest.is_empty() {
        true => home,
        false => home.join(rest),
    })
}

// Persistent, machine-generated data: the sessions database and the browser profile. `$XDG_DATA_HOME/mypi` on unix, `%LOCALAPPDATA%\mypi` on windows.
pub fn data_dir() -> PathBuf {
    match os::current().family {
        Family::Unix => base("XDG_DATA_HOME", ".local/share").join("mypi"),
        Family::Windows => app_dir("LOCALAPPDATA", ["AppData", "Local"]).join("mypi"),
    }
}

// Configuration the user edits: `$XDG_CONFIG_HOME/mypi` on unix, `%APPDATA%\mypi` on windows. A missing config.yaml is written here.
pub fn config_dir() -> PathBuf {
    match os::current().family {
        Family::Unix => base("XDG_CONFIG_HOME", ".config").join("mypi"),
        Family::Windows => app_dir("APPDATA", ["AppData", "Roaming"]).join("mypi"),
    }
}

// The user's download folder: the desktop's own answer, else `~/Downloads`.
pub fn downloads_dir() -> Option<PathBuf> {
    match os::current().family {
        Family::Unix => xdg_download(),
        Family::Windows => home().map(|home| home.join("Downloads")),
    }
}

// `XDG_DOWNLOAD_DIR` from the user-dirs file; `~/Downloads` when it names nothing.
fn xdg_download() -> Option<PathBuf> {
    let home = home()?;
    let dirs = var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".config"))
        .join("user-dirs.dirs");
    if let Ok(text) = std::fs::read_to_string(&dirs) {
        for line in text.lines() {
            let Some(rest) = line.trim().strip_prefix("XDG_DOWNLOAD_DIR=") else {
                continue;
            };
            let raw = rest.trim().trim_matches('"');
            let named = match raw.strip_prefix("$HOME") {
                Some(tail) => home.join(tail.trim_start_matches('/')),
                None => PathBuf::from(raw),
            };
            if !raw.is_empty() {
                return Some(named);
            }
        }
    }
    Some(home.join("Downloads"))
}

// Scratch space: the system's per-user temporary directory.
pub fn temp_dir() -> PathBuf {
    std::env::temp_dir()
}

// One environment variable, as UTF-8. `None` = unset or not text.
pub fn var(name: &str) -> Option<String> {
    std::env::var(name).ok()
}

// One environment variable, uninterpreted. Paths come through here: a directory name need not be valid UTF-8. Empty counts as unset.
pub fn var_os(name: &str) -> Option<std::ffi::OsString> {
    std::env::var_os(name).filter(|v| !v.is_empty())
}

// An XDG base directory: the variable if it is set, else the specification's default under the home directory.
fn base(variable: &str, fallback: &str) -> PathBuf {
    if let Some(dir) = var_os(variable) {
        return PathBuf::from(dir);
    }
    match home() {
        Some(home) => home.join(fallback),
        None => PathBuf::from("."),
    }
}

// A Windows per-user folder: the environment variable, else the documented path under home.
fn app_dir(variable: &str, fallback: [&str; 2]) -> PathBuf {
    if let Some(dir) = var_os(variable) {
        return PathBuf::from(dir);
    }
    match home() {
        Some(home) => home.join(fallback[0]).join(fallback[1]),
        None => PathBuf::from(fallback[0]).join(fallback[1]),
    }
}
