//! Where every binary mode fans out: the two processes, and the one-shot queries.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::Context;

use mypi::cli::Cli;
use mypi::ipc::contract::Viewer;
use mypi::ipc::endpoint::{self, Endpoint};
use mypi::ipc::server;
use mypi::server::api::Api;
use mypi::server::config::Config;
use mypi::server::platform::time;
use mypi::server::storage;
use mypi::ui::session::Switches;
use mypi::ui::tui;

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse(std::env::args().skip(1))?;
    let db = storage::default_path();

    if cli.server {
        return run_server(&db);
    }
    if cli.sessions {
        return mypi::oneshot::run_sessions(&Endpoint::file_path(&db));
    }
    if let Some(file) = std::env::var_os("MYPI_ENDPOINT") {
        return front_end(PathBuf::from(file), &cli);
    }
    if let Ok(endpoint) = endpoint::read(&Endpoint::file_path(&db)) {
        return front_end_with(endpoint, &cli);
    }

    let cfg = Config::load()?;
    let kind = cfg.app.server.platform.ipc.cost();
    let endpoint = Endpoint::for_db(&db, kind);
    let file = Endpoint::file_path(&db);
    let Some(claim) = server::claim(&db)? else {
        anyhow::bail!("another mypi owns {}", db.display());
    };
    time::init(cfg.app.server.platform.timezone.as_deref());
    let api = Arc::new(Api::new(cfg));
    let bound = server::bind(api, &db, endpoint, claim)?;
    let mut front = std::process::Command::new(std::env::current_exe()?)
        .env("MYPI_ENDPOINT", &file)
        .spawn()
        .context("cannot start the front end")?;
    server::serve(bound)?;
    let _ = front.wait();
    Ok(())
}

/// `mypi --server`: the server alone.
fn run_server(db: &Path) -> anyhow::Result<()> {
    let cfg = Config::load()?;
    let Some(claim) = server::claim(db)? else {
        anyhow::bail!("another mypi owns {}", db.display());
    };
    time::init(cfg.app.server.platform.timezone.as_deref());
    let kind = cfg.app.server.platform.ipc.cost();
    let endpoint = Endpoint::for_db(db, kind);
    server::run(Api::new(cfg), db, endpoint, claim)
}

/// The front end's own process, once the server is reachable.
fn front_end(file: PathBuf, cli: &Cli) -> anyhow::Result<()> {
    let endpoint =
        endpoint::read(&file).with_context(|| format!("cannot read {}", file.display()))?;
    front_end_with(endpoint, cli)
}

fn front_end_with(endpoint: Endpoint, cli: &Cli) -> anyhow::Result<()> {
    let cwd = std::env::current_dir().context("cannot read the current directory")?;
    let session =
        mypi::ui::session::Session::connect(endpoint, cwd.clone(), Switches::default(), viewer())?;
    let model = cli.model.as_deref().and_then(model_id);
    let startup = if let Some(id) = cli.attach {
        tui::app::Startup::attach(cwd, id, model)
    } else if cli.resume {
        let mut startup = tui::app::Startup::resume(cwd);
        startup.model = model;
        startup
    } else {
        tui::app::Startup::main(cwd, model)
    };
    tui::run(session, startup)
}

fn model_id(spec: &str) -> Option<mypi::ipc::contract::ModelId> {
    let (provider, model) = spec.split_once(':')?;
    Some(mypi::ipc::contract::ModelId {
        provider: provider.to_string(),
        model: model.to_string(),
    })
}

fn viewer() -> Viewer {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    Viewer(format!("{}-{now}", std::process::id()))
}
