use std::sync::Arc;

use rusqlite::Connection;

use super::Live;
use super::artifacts::ArtifactStore;
use crate::server::agent::request;
use crate::server::agent::tools::{self, BashEnv, Env, Inbox, Window, Workspace};

/// Build the tool environment for one turn.
pub fn build(r: &Connection, live: &mut Live, session: i64) -> Env {
    let cwd = live.driver.cwd().to_path_buf();
    let root = live.driver.workspace.root().to_path_buf();
    let mut workspace = Workspace::new(root);
    workspace.chdir(cwd.clone());

    if live.artifacts.is_none() {
        live.artifacts = r.path().map(|db| {
            let arts: Arc<dyn tools::Artifacts> =
                Arc::new(ArtifactStore::new(std::path::PathBuf::from(db)));
            arts
        });
    }

    let mut env = Env::new(cwd);
    env.workspace = workspace;
    env.limits = live.settings.limits;
    env.browser = live.settings.browser.clone();
    env.enabled = live.settings.enabled.clone();
    env.mcp = live.mcp.clone();
    env.bash = BashEnv {
        timeout: live.settings.bash_timeout,
        interrupt: Some(live.stop.clone()),
    };
    env.inbox = Inbox {
        session,
        todo: todo_initial(r, session),
        jobs: live.jobs.clone(),
        window: window(r, session),
        artifacts: live.artifacts.clone(),
        ..Inbox::default()
    };
    env
}

/// The todo workbench: rebuilt from the session's blocks at every turn.
fn todo_initial(r: &Connection, session: i64) -> Vec<tools::TodoPhase> {
    let Ok((on, cwd)) = r.query_row(
        "SELECT todo_on, cwd FROM sessions WHERE id = ?1",
        [session],
        |r| Ok((r.get::<_, i64>(0)? != 0, r.get::<_, Option<String>>(1)?)),
    ) else {
        return Vec::new();
    };
    tools::todo_replay(r, session, on, cwd).unwrap_or_default()
}

/// The context tool's window.
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
