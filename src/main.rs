//! Where every binary mode fans out: the two processes, and the one-shot queries.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Context;

use mypi::cli::Cli;
use mypi::ipc::endpoint::{self, Endpoint, Kind};
use mypi::ipc::server;
use mypi::server::api::Api;
use mypi::server::config::{
    Config, KEY_CONFIG_SERVER_PLATFORM_IPC, KEY_CONFIG_SERVER_PLATFORM_TIMEZONE,
};
use mypi::server::platform::time;
use mypi::server::storage;
use mypi::ui::tui;

fn main() -> anyhow::Result<()> {
    // The worker is this same executable, told by its environment: no flag, no config, no db.
    if std::env::var_os(mypi::server::agent::request::transport::worker::WORKER_ENV).is_some() {
        return mypi::server::agent::request::transport::worker::run();
    }
    let cli = Cli::parse(std::env::args().skip(1))?;
    let db = storage::default_path();

    if cli.server {
        return run_server(&db);
    }
    if let Some(file) = std::env::var_os("MYPI_ENDPOINT") {
        return front_end(PathBuf::from(file), &cli);
    }
    if let Ok(endpoint) = endpoint::read(&Endpoint::file_path(&db)) {
        return front_end_with(endpoint, &cli);
    }

    let cfg = Config::load()?;
    let endpoint = Endpoint::for_db(&db, ipc_kind(&cfg));
    let file = Endpoint::file_path(&db);
    let Some(claim) = server::claim(&db)? else {
        anyhow::bail!("another mypi owns {}", db.display());
    };
    init_time(&cfg);
    let api = Arc::new(Api::open(db.clone(), cfg)?);
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
    init_time(&cfg);
    let endpoint = Endpoint::for_db(db, ipc_kind(&cfg));
    server::run(Api::open(db.to_path_buf(), cfg)?, db, endpoint, claim)
}

/// The endpoint kind this process asks for.
fn ipc_kind(cfg: &Config) -> Kind {
    match cfg
        .get(KEY_CONFIG_SERVER_PLATFORM_IPC, None)
        .ok()
        .and_then(|value| value.as_text().map(str::to_string))
        .as_deref()
    {
        Some("unix") => Kind::Unix,
        Some("pipe") => Kind::Pipe,
        _ => Kind::of_this_system(),
    }
}

/// The time zone by name; an unset key leaves the system's.
fn init_time(cfg: &Config) {
    let zone = cfg
        .get(KEY_CONFIG_SERVER_PLATFORM_TIMEZONE, None)
        .ok()
        .and_then(|value| value.as_text().map(str::to_string));
    time::init(zone.as_deref());
}

/// The front end's own process, once the server is reachable.
fn front_end(file: PathBuf, cli: &Cli) -> anyhow::Result<()> {
    let endpoint =
        endpoint::read(&file).with_context(|| format!("cannot read {}", file.display()))?;
    front_end_with(endpoint, cli)
}

fn front_end_with(endpoint: Endpoint, cli: &Cli) -> anyhow::Result<()> {
    let cwd = std::env::current_dir().context("cannot read the current directory")?;
    let session = mypi::ui::session::Session::connect(endpoint)?;
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

fn model_id(spec: &str) -> Option<mypi::ui::tui::zone::ModelId> {
    let (provider, model) = spec.split_once(':')?;
    Some(mypi::ui::tui::zone::ModelId {
        provider: provider.to_string(),
        model: model.to_string(),
    })
}
