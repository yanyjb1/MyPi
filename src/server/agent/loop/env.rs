//! Build the Env tools need: workspace, output budget, inbox, bash's two, profile gate.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Context, Result};
use rusqlite::Connection;
use rusqlite::OptionalExtension;

use super::Live;
use crate::server::agent::request;
use crate::server::agent::tools::{self, BashEnv, Env, Inbox, Window, Workspace};
use crate::server::storage::decode::{ARTIFACT_COLS, artifact_from_row};
use crate::server::storage::{Reader, base36};

/// Build the Env one turn needs. The working directory is read out by the front end and handed in.
pub fn build(r: &Connection, live: &mut Live, session: i64) -> Env {
    // Move the whole workspace in: `root` (where the session started) and every `cd` step must
    // survive, so a freshly built one cannot take its place.
    let cwd = live.workspace.cwd().to_path_buf();
    let workspace = std::mem::replace(&mut live.workspace, Workspace::new(cwd.as_path()));

    // Artifact fetch: built on the session's first Env and kept on it — the temp files follow the
    // session's `Live`.
    if live.artifacts.is_none() {
        live.artifacts = r.path().map(|db| {
            let arts: Arc<dyn tools::Artifacts> = Arc::new(Arts::new(PathBuf::from(db)));
            arts
        });
    }

    let mut env = Env::new(cwd.as_path());
    env.workspace = workspace;
    env.limits = live.settings.limits;
    env.browser = live.settings.browser.clone();
    env.enabled = live.settings.enabled.clone();
    env.bash = BashEnv {
        timeout: live.settings.bash_timeout,
        interrupt: Some(live.stop.clone()),
    };
    env.inbox = Inbox {
        todo: std::mem::take(&mut live.todo),
        window: window(r, session),
        artifacts: live.artifacts.clone(),
    };
    env
}

/// After the turn: put back the two things tools changed.
pub fn restore(live: &mut Live, env: Env) {
    live.workspace = env.workspace;
    live.todo = env.inbox.todo;
}

/// The window `context` needs: the messages settled so far, plus the `cwd` moves. The context itself
/// comes from request.
fn window(r: &Connection, session: i64) -> Window {
    let messages = request::rebuild(r, session).unwrap_or_default();
    let trail: Vec<(i64, String)> = r
        .prepare("SELECT turn_id, cwd FROM cwd_history WHERE session_id = ?1 ORDER BY turn_id")
        .and_then(|mut stmt| {
            stmt.query_map([session], |r| {
                Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<(i64, String)>>>()
        })
        .unwrap_or_default();
    Window {
        messages: Arc::new(messages),
        cwd_trail: Arc::new(trail),
    }
}

/// Artifact fetch: block id in, one temp path out (written under `<temp>/mypi-arts/<ref>`). The temp
/// files live with the session's `Live`.
struct Arts {
    /// The database file: each fetch opens its own read connection (one connection per thread).
    db: PathBuf,
    /// The paths this turn materialized.
    made: Mutex<Vec<PathBuf>>,
}

impl Arts {
    fn new(db: PathBuf) -> Self {
        Self {
            db,
            made: Mutex::new(Vec::new()),
        }
    }

    /// The artifact basket (under the system temp dir).
    fn dir() -> PathBuf {
        crate::server::platform::env::temp_dir().join("mypi-arts")
    }
}

impl tools::Artifacts for Arts {
    fn path_for(&self, id: i64) -> Result<PathBuf> {
        let dir = Self::dir();
        let path = dir.join(base36(id));
        if path.exists() {
            return Ok(path);
        }
        let conn = Reader::open(&self.db)?;
        let row = conn
            .query_row(
                &format!("SELECT {ARTIFACT_COLS} FROM artifacts WHERE block_id = ?1"),
                [id],
                artifact_from_row,
            )
            .optional()?;
        Reader::close(conn);
        let artifact = row.with_context(|| format!("no artifact is bound to block {id}"))?;
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("cannot make the artifact directory {}", dir.display()))?;
        write_temp(&path, &artifact.content)?;
        self.made
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .push(path.clone());
        Ok(path)
    }
}

impl Drop for Arts {
    fn drop(&mut self) {
        let made = std::mem::take(&mut *self.made.lock().unwrap_or_else(|e| e.into_inner()));
        for path in made {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Drop one read-only artifact text on disk.
fn write_temp(path: &Path, content: &str) -> Result<()> {
    std::fs::write(path, content)
        .with_context(|| format!("cannot write the artifact {}", path.display()))?;
    Ok(())
}
