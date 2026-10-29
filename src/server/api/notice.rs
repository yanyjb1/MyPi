//! The server's own outbound path: a notice belongs to one session and reaches its stream.

use anyhow::{Result, anyhow};

use crate::ipc::contract::Notice;
use crate::server::api::driver;

pub use crate::ipc::contract::NoticeLevel;

/// Send one notice to a session's stream; nobody on it, it is dropped.
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
