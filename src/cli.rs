//! CLI argument parsing — hand-rolled, no clap (dep policy). The four shapes a
//! command line can take, and the flags, are spelled out in `USAGE` below.

/// Parsed command line.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Cli {
    /// Run the server alone (--server).
    pub server: bool,
    pub attach: Option<i64>,
    /// Open the resume picker immediately (--resume).
    pub resume: bool,
    /// Model override for this session (`provider:id`).
    pub model: Option<String>,
}

const USAGE: &str = "\
mypi — coding agent (server + front end)

Usage:
  mypi [options]             the whole product: a server if none is running,
                             then a front end on this terminal
  mypi attach <id>           attach the front end to a stored session
  mypi --server              run the server alone (it exits when the last
                             front end is gone)

Options:
  -h, --help        Print this help and exit
  -V, --version     Print version and exit
  --resume          Open the session-resume picker on startup
  --model <p:id>    Session model override (provider:id, not persisted)

Configuration: ~/.config/mypi/config.yaml (see models_example.yml in the repo).
Sessions:      ~/.local/share/mypi/history.db
Server socket: $XDG_RUNTIME_DIR/mypi-<hash>.sock (one per database)";

impl Cli {
    /// Parse argv (excluding argv[0]). Unknown flags and stray positionals
    /// are errors — silent acceptance breeds typos like `--resune`.
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
