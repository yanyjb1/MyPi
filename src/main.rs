// Where every binary mode fans out: the two processes, the HTTP door, and the one-shot queries.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use anyhow::Context;

use mypi::cli::Cli;
use mypi::ipc::endpoint::{self, Endpoint, Kind};
use mypi::ipc::server;
use mypi::platform::proc;
use mypi::platform::time;
use mypi::server::api::Api;
use mypi::server::config::{
    Config, KEY_CONFIG_SERVER_PLATFORM_IPC, KEY_CONFIG_SERVER_PLATFORM_TIMEZONE,
};
use mypi::server::storage;
use mypi::ui::bridge;
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
    let cfg = Config::load()?;
    init_time(&cfg);
    init_home(&cfg);
    let file = Endpoint::file_path(&db);
    if cli.bridge {
        return run_bridge(&cfg, &cli, &file);
    }
    // A leftover endpoint file whose server is gone is no reason to refuse to start.
    if let Ok(running) = endpoint::read(&file)
        && let Ok(session) = mypi::ui::session::Session::open(running, None)
    {
        return run(session, &cli);
    }
    let endpoint = spawn_server(&file)?;
    let session = mypi::ui::session::Session::open(endpoint, None)?;
    run(session, &cli)
}

// The server this endpoint file names, started when nobody answers there yet.
fn ensure_server(file: &Path) -> anyhow::Result<Endpoint> {
    if let Ok(running) = endpoint::read(file) {
        return Ok(running);
    }
    spawn_server(file)
}

// The server is a background child: it takes no terminal, prints nothing, and leaves with the last front end.
fn spawn_server(file: &Path) -> anyhow::Result<Endpoint> {
    let mut cmd = std::process::Command::new(std::env::current_exe()?);
    cmd.arg("--server")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    proc::Process::spawn_detached(proc::Origin::Frontend, &mut cmd)
        .context("cannot start the server")?;
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(endpoint) = endpoint::read(file) {
            return Ok(endpoint);
        }
        if Instant::now() >= deadline {
            anyhow::bail!("the server did not answer at {}", file.display());
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

// A server has no terminal to complain on: its last words land beside the launch directory.
fn log_panics(dir: PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        use std::io::Write;
        let backtrace = std::backtrace::Backtrace::force_capture();
        let stamp = time::now_stamp();
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dir.join("mypi-server.log"))
        {
            let _ = writeln!(file, "{stamp} {info}\n{backtrace}\n");
        }
        previous(info);
    }));
}

// `mypi --server`: the server alone.
fn run_server(db: &Path) -> anyhow::Result<()> {
    if let Ok(cwd) = std::env::current_dir() {
        log_panics(cwd);
    }
    let cfg = Config::load()?;
    let Some(claim) = server::claim(db)? else {
        anyhow::bail!("another mypi owns {}", db.display());
    };
    init_time(&cfg);
    init_home(&cfg);
    let endpoint = Endpoint::for_db(db, ipc_kind(&cfg));
    let served = server::run(Api::open(db.to_path_buf(), cfg)?, db, endpoint, claim);
    #[cfg(feature = "web")]
    mypi::web::shutdown();
    served
}

// `mypi bridge [--lan]`: the HTTP door for every front end that is not this terminal. The list of
// adapters is the global one and nothing else: a bridge with no list serves nobody, so it refuses
// to start rather than guess.
fn run_bridge(cfg: &Config, cli: &Cli, file: &Path) -> anyhow::Result<()> {
    let root = cfg
        .paths()
        .global_root()
        .context("no global config directory to read adapters.yml from")?
        .to_path_buf();
    let manifest =
        bridge::Manifest::load(&root.join(bridge::FILE)).context("the bridge cannot start")?;
    let webui = bridge::Webui::open(&root);
    let endpoint = ensure_server(file)?;
    bridge::run(bridge::Options {
        endpoint,
        manifest,
        webui,
        lan: cli.lan,
    })
}

// The endpoint kind this process asks for.
fn ipc_kind(cfg: &Config) -> Kind {
    let value = cfg
        .get(KEY_CONFIG_SERVER_PLATFORM_IPC, None)
        .expect("config.server.platform.ipc must be readable");
    let name = value
        .as_text()
        .expect("config.server.platform.ipc must be text");
    let Some(kind) = Kind::from_name(name) else {
        panic!(
            "config.server.platform.ipc: `{name}` is not one of {}",
            Kind::NAMES.join(", ")
        )
    };
    kind
}

// The time zone by name; an unset key leaves the system's.
fn init_time(cfg: &Config) {
    let zone = cfg
        .get(KEY_CONFIG_SERVER_PLATFORM_TIMEZONE, None)
        .ok()
        .and_then(|value| value.as_text().map(str::to_string));
    time::init(zone.as_deref());
}

// What `~` stands for: the configured directory, else nothing and the platform's own home answers.
fn init_home(cfg: &Config) {
    mypi::platform::env::anchor(cfg.home());
}

// The front end on an open connection: read the launch directory, pick the first zone, go.
fn run(session: mypi::ui::session::Session, cli: &Cli) -> anyhow::Result<()> {
    let cwd = std::env::current_dir().context("cannot read the current directory")?;
    let model = cli.model.clone();
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
