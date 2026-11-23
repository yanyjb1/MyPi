// CLI argument parsing, hand-rolled: the shapes a command line can take are `USAGE` below.

// Parsed command line.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Cli {
    // Run the server alone (--server).
    pub server: bool,
    pub attach: Option<i64>,
    // Open the resume picker immediately (--resume).
    pub resume: bool,
    // Model override for this session (`provider:id`).
    pub model: Option<String>,
    // Serve the HTTP door and the webui instead of the terminal (--bridge).
    pub bridge: bool,
    // Let that door be reached from the local network (--lan).
    pub lan: bool,
}

const USAGE: &str = "\
mypi — coding agent (server + front end)

Usage:
  mypi [options]             the whole product: a server if none is running,
                             then a front end on this terminal
  mypi attach <id>           attach the front end to a stored session
  mypi --server              run the server alone (it exits when the last
                             front end is gone)
  mypi --bridge [--lan]      run the HTTP door for other front ends: the
                             webui is there the moment one connects

Options:
  -h, --help        Print this help and exit
  -V, --version     Print version and exit
  --resume          Open the session-resume picker on startup
  --model <p:id>    Session model override (provider:id, not persisted)
  --lan             With `--bridge`: listen on the local network, not just this machine

Configuration: ~/.config/mypi/config.yaml (see models_example.yml in the repo).
Sessions:      ~/.local/share/mypi/history.db
Bridge:        ~/.config/mypi/adapters.yml lists the front ends the door serves
Server socket: $XDG_RUNTIME_DIR/mypi-<hash>.sock (one per database)";

impl Cli {
    // Parse argv (excluding argv[0]); an unknown flag or a stray word is an error, so a typo like
    // `--resune` is said out loud instead of silently ignored.
    pub fn parse<I: IntoIterator<Item = String>>(args: I) -> anyhow::Result<Self> {
        let mut cli = Cli::default();
        let mut it = args.into_iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "-h" | "--help" => {
                    println!("{USAGE}");
                    std::process::exit(0);
                }
                "-V" | "--version" => {
                    println!("mypi {}", env!("CARGO_PKG_VERSION"));
                    std::process::exit(0);
                }
                "--server" => cli.server = true,
                "--resume" => cli.resume = true,
                "--lan" => cli.lan = true,
                "--bridge" => cli.bridge = true,
                "--model" => {
                    let v = it.next().ok_or_else(|| {
                        anyhow::anyhow!("--model needs a value: --model <provider:id>")
                    })?;
                    if !v.contains(':') {
                        anyhow::bail!("--model expects <provider>:<id>, got `{v}`");
                    }
                    cli.model = Some(v);
                }
                "attach" => {
                    let v = it.next().ok_or_else(|| {
                        anyhow::anyhow!("attach needs a session id: mypi attach <id>")
                    })?;
                    cli.attach = Some(v.parse().map_err(|_| {
                        anyhow::anyhow!("attach expects a numeric session id, got `{v}`")
                    })?);
                }
                other => anyhow::bail!("unknown argument `{other}` — see `mypi --help`"),
            }
        }
        Ok(cli)
    }
}
