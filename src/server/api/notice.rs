//! The server's own outbound path: a notice belongs to one session and reaches its watchers.

use anyhow::{Result, anyhow};

use crate::server::api::driver;

pub use crate::ipc::contract::{Notice, NoticeLevel};

/// Send one notice to a session's watchers; with none, queue it for the next watch.
pub fn notify(session: i64, level: NoticeLevel, content: impl Into<String>) -> Result<()> {
    let state = driver::state_ready().ok_or_else(|| anyhow!("api state is not initialized"))?;
    let state = state.lock().unwrap_or_else(|e| e.into_inner());
    state.relay.notice(Notice {
        session,
        level,
        content: content.into(),
    });
    Ok(())
}
