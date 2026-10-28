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
        memo: live.memo.clone(),
        todo: live.todo.clone(),
        jobs: live.jobs.clone(),
        subagent: live.subagent.clone(),
        window: window(r, session),
        artifacts: live.artifacts.clone(),
        ..Inbox::default()
    };
    env
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
