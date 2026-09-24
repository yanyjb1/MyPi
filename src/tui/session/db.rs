//! Session store location + legacy-DB migration.

// Database location (XDG Data spec): $XDG_DATA_HOME/mypi/sessions.db,
// i.e. ~/.local/share/mypi/sessions.db by default. Legacy layout support:
// `~/.local/share/mypi` used to be the SQLite file itself — renamed to
// sessions.db on first open of the new layout.
pub(crate) fn db_path() -> std::path::PathBuf {
    crate::xdg::data_dir().join("sessions.db")
}

/// Migrate the legacy database file (a bare `mypi` file under
/// ~/.local/share) to the canonical `mypi/sessions.db` layout. No-op when
/// the legacy file is absent or already migrated.
pub(crate) fn migrate_legacy_db(base: &std::path::Path) {
    let legacy = base.join("mypi");
    if !legacy.is_file() {
        return;
    }
    // The legacy file **occupies the directory's name**, so: move the db
    // and its WAL/SHM companions out of the way, create the real
    // directory, then move them in as `sessions.db*`.
    let stash = base.join(".mypi-legacy-migrate");
    let _ = std::fs::remove_dir_all(&stash);
    std::fs::create_dir_all(&stash).ok();
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::rename(
            base.join(format!("mypi{suffix}")),
            stash.join(format!("db{suffix}")),
        );
    }
    if let Err(e) = std::fs::create_dir_all(base.join("mypi")) {
        eprintln!("mypi: cannot create data dir: {e}");
        return;
    }
    for suffix in ["", "-wal", "-shm"] {
        let _ = std::fs::rename(
            stash.join(format!("db{suffix}")),
            base.join(format!("mypi/sessions.db{suffix}")),
        );
    }
    let _ = std::fs::remove_dir_all(&stash);
}

// Session name for the statusline: an explicit /name wins; otherwise
// one is synthesized — first 7 chars of the first user message within the session.
