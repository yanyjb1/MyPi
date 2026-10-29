//! Where the database lives.

use std::path::PathBuf;

/// The sessions database's file name.
pub const DB_FILE: &str = "history.db";

/// Where the sessions database lives by default: the platform's data directory plus [`DB_FILE`].
pub fn default_path() -> PathBuf {
    crate::server::platform::env::data_dir().join(DB_FILE)
}
