//! `mypi sessions` / `mypi replay` — one-shot queries through the daemon
//! socket. They connect (spawning a headless daemon if none runs), ask,
//! print tab-separated answers, and quit.

use crate::server::daemon::Daemon;
use crate::server::hub::{SessionHub, SessionSpec};
use crate::server::wire::{ClientMsg, ServerMsg};
use crate::server::{socket_path, ClientConn};

/// Connect to the daemon at the standard socket, starting one if absent.
fn ensure_daemon(spec: &SessionSpec) -> anyhow::Result<ClientConn> {
    let path = socket_path();
    if let Ok(c) = ClientConn::connect(&path) {
        return Ok(c);
    }
    let hub = SessionHub::new(crate::xdg::data_dir().join("sessions.db3"));
    let daemon = Daemon::bind(
        path.clone(),
        hub,
        spec.clone(),
        std::time::Duration::from_secs(600),
    )?;
    std::thread::spawn(move || {
        let _ = daemon.serve();
    });
    // Wait for the socket to accept.
    for _ in 0..200 {
        if let Ok(c) = ClientConn::connect(&path) {
            return Ok(c);
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    anyhow::bail!("daemon socket never became ready: {}", path.display())
}

/// `mypi sessions`: TSV rows — id, name, started, cwd.
pub fn run_sessions(spec: &SessionSpec) -> anyhow::Result<()> {
    let mut conn = ensure_daemon(spec)?;
    conn.request(&ClientMsg::ListSessions { under: None })?;
    let reply = conn.wait_for(|m| matches!(m, ServerMsg::Sessions { .. }))?;
    conn.quit()?;
    let ServerMsg::Sessions { sessions } = reply else {
        anyhow::bail!("unexpected reply for sessions");
    };
    println!("id\tname\tstarted\tcwd");
    for s in sessions {
        println!(
            "{}\t{}\t{}\t{}",
            s.id,
            s.name.as_deref().unwrap_or(""),
            s.started_at,
            s.cwd.as_deref().unwrap_or(""),
        );
    }
    Ok(())
}

/// `mypi replay <id> <round>`: pretty-print the rebuilt request (stderr-safe
/// machine format: one JSON object on stdout).
pub fn run_replay(spec: &SessionSpec, id: i64, round: i64) -> anyhow::Result<()> {
    let mut conn = ensure_daemon(spec)?;
    conn.request(&ClientMsg::Replay { id, round })?;
    let reply = conn.wait_for(|m| matches!(m, ServerMsg::Replay { .. } | ServerMsg::Error { .. }))?;
    conn.quit()?;
    let ServerMsg::Replay { replay, .. } = reply else {
        anyhow::bail!("replay failed: {reply:?}");
    };
    println!(
        "{}",
        serde_json::to_string_pretty(&replay).unwrap_or_else(|_| "{}".into())
    );
    Ok(())
}
