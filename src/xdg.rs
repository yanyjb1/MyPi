//! XDG base directories, shared by every subsystem that persists anything.
//!
//! One definition (this file) so the database, the browser profile and future
//! stores agree on the same roots — the spec's whole point. Paths resolve
//! eagerly at call time; nothing here touches the filesystem.

/// `$XDG_DATA_HOME` (default `~/.local/share`): persistent, machine-generated
/// data the user expects to survive reboots — the sessions database, the
/// browser profile's cookies/extensions.
pub fn data_base() -> std::path::PathBuf {
    std::env::var_os("XDG_DATA_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .map(|h| h.join(".local/share"))
        })
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

/// `$XDG_CONFIG_HOME` (default `~/.config`): user-authored configuration.
/// Kept next to `data_base` for symmetry even though `ai::config` grew its
/// own copy first — that one migrates here when it next changes anyway.
pub fn config_base() -> std::path::PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(std::path::PathBuf::from)
        .or_else(|| {
            std::env::var_os("HOME")
                .map(std::path::PathBuf::from)
                .map(|h| h.join(".config"))
        })
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

/// mypi's slice of the data home: `$XDG_DATA_HOME/mypi`.
pub fn data_dir() -> std::path::PathBuf {
    data_base().join("mypi")
}

/// Persistent browser profile: `$XDG_DATA_HOME/mypi/browser/profile`.
///
/// Chromium state (cookies, login sessions, extensions) is data, not config
/// and not cache — cache would get swept by system cleanup tools and take
/// the logins with it. Overridable via `MYPI_BROWSER_PROFILE_DIR` for tests
/// and parallel sessions; a missing HOME falls back to a scratch directory
/// inside [`std::env::temp_dir`] so a browser can still launch.
pub fn browser_profile_dir() -> std::path::PathBuf {
    if let Some(dir) = std::env::var_os("MYPI_BROWSER_PROFILE_DIR")
        && !dir.is_empty()
    {
        return std::path::PathBuf::from(dir);
    }
    let base = std::env::var_os("HOME")
        .map(|_| data_dir())
        .unwrap_or_else(std::env::temp_dir);
    base.join("browser").join("profile")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// All env-mutating tests take this lock: `set_var`/`remove_var` are
    /// process-global, and cargo runs this module's tests on several threads.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_env<F: FnOnce()>(vars: &[(&str, Option<&str>)], f: F) {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        // Rust 2024 marks env mutation unsafe (process-global state).
        //
        // SAFETY: env vars are process-global, so concurrent tests touching
        // them race. ENV_LOCK serializes every env-mutating test in this
        // binary; restoration always runs (no early return between save and
        // restore), so even a panicking test leaves the env as it found it.
        unsafe {
            let saved: Vec<(String, Option<std::ffi::OsString>)> = vars
                .iter()
                .map(|(k, _)| (k.to_string(), std::env::var_os(k)))
                .collect();
            for (k, v) in vars {
                match v {
                    Some(val) => std::env::set_var(k, val),
                    None => std::env::remove_var(k),
                }
            }
            f();
            for (k, old) in saved {
                match old {
                    Some(val) => std::env::set_var(k, val),
                    None => std::env::remove_var(k),
                }
            }
        }
    }

    #[test]
    fn profile_dir_prefers_the_explicit_override() {
        with_env(
            &[
                ("MYPI_BROWSER_PROFILE_DIR", Some("/tmp/explicit-profile")),
                ("XDG_DATA_HOME", Some("/tmp/xdg-data")),
                ("HOME", Some("/tmp/fake-home")),
            ],
            || {
                assert_eq!(
                    browser_profile_dir(),
                    std::path::PathBuf::from("/tmp/explicit-profile")
                );
            },
        );
    }

    #[test]
    fn profile_dir_follows_xdg_data_home() {
        with_env(
            &[
                ("MYPI_BROWSER_PROFILE_DIR", None),
                ("XDG_DATA_HOME", Some("/tmp/xdg-data")),
            ],
            || {
                assert_eq!(
                    browser_profile_dir(),
                    std::path::PathBuf::from("/tmp/xdg-data/mypi/browser/profile")
                );
            },
        );
    }

    #[test]
    fn profile_dir_falls_back_to_home_layout() {
        with_env(
            &[
                ("MYPI_BROWSER_PROFILE_DIR", None),
                ("XDG_DATA_HOME", None),
                ("HOME", Some("/tmp/fake-home")),
            ],
            || {
                assert_eq!(
                    browser_profile_dir(),
                    std::path::PathBuf::from("/tmp/fake-home/.local/share/mypi/browser/profile")
                );
            },
        );
    }

    #[test]
    fn data_dir_is_mypi_under_the_data_base() {
        with_env(&[("XDG_DATA_HOME", Some("/tmp/xdg-data"))], || {
            assert_eq!(data_dir(), std::path::PathBuf::from("/tmp/xdg-data/mypi"));
        });
    }
}
