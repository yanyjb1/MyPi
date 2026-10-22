//! Where every binary mode fans out: the two processes, and the one-shot
//! queries. The modes are the ones `mypi --help` lists.

use anyhow::Context;

use mypi::cli::Cli;
use mypi::ipc;
use mypi::ipc::client::Client;
use mypi::ipc::server;
use mypi::server::config::Config;
use mypi::server::platform::ipc::Cost;
use mypi::server::platform::time;
use mypi::server::storage;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse(std::env::args().skip(1))?;
    let cfg = Config::load()?;
    let db = storage::default_path();

    if cli.server {
        return run_server(cfg, &db);
    }
    if cli.sessions {
        return mypi::oneshot::run_sessions(&cfg);
    }
    run(cfg, &db)
}

/// `mypi --server`: the server alone. Losing the lock means somebody else already serves this database.
fn run_server(cfg: Config, db: &std::path::Path) -> anyhow::Result<()> {
    let Some(claim) = server::claim(db)? else {
        anyhow::bail!("another mypi owns {}", db.display());
    };
    time::init(cfg.app.server.platform.timezone.as_deref());
    server::run(cfg, db, claim)
}

/// `mypi`: a server if there is none, then a front end on this terminal.
fn run(cfg: Config, db: &std::path::Path) -> anyhow::Result<()> {
    let cost = cfg.app.server.platform.ipc.cost();
    let path = ipc::socket_path(db);

    // Somebody is serving this database already: this process is a front end.
    if Client::connect(cost, &path, ipc::viewer()).is_ok() {
        return front_end(cfg, &path, cost);
    }

    if let Some(claim) = server::claim(db)? {
        // Nobody was serving and the lock is ours: become the server, then start the front end.
        time::init(cfg.app.server.platform.timezone.as_deref());
        let bound = server::bind(cfg.clone(), db, claim)?;
        let mut front = std::process::Command::new(std::env::current_exe()?)
            .spawn()
            .context("cannot start the front end")?;
        server::serve(bound)?;
        let _ = front.wait();
        return Ok(());
    }

    // Somebody else won the race and is starting up: wait for their socket.
    wait_for_front_end(cfg, &path, cost)
}

/// Wait for the socket of a server that is starting up, then be a front end.
fn wait_for_front_end(cfg: Config, path: &std::path::Path, cost: Cost) -> anyhow::Result<()> {
    for _ in 0..200 {
        if Client::connect(cost, path, ipc::viewer()).is_ok() {
            return front_end(cfg, path, cost);
        }
        std::thread::sleep(std::time::Duration::from_millis(25));
    }
    anyhow::bail!(
        "a mypi server is starting up but never answered on {}",
        path.display()
    )
}

/// The front end's own process, once the server is reachable. The terminal side is the next wave.
fn front_end(cfg: Config, path: &std::path::Path, _cost: Cost) -> anyhow::Result<()> {
    time::init(cfg.app.server.platform.timezone.as_deref());
    anyhow::bail!(
        "connected to the server at {} — the front end itself is the next wave",
        path.display()
    )
}
