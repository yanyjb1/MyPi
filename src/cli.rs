//! CLI argument parsing — hand-rolled, no clap (dep policy).
//!
//! Two forms:
//!   mypi [options]            the TUI (connects to the daemon, spawning one
//!                             if none is running)
//!   mypi --server             run the daemon in the foreground and exit
//!                             on idle / quit
//!   mypi attach <id>          attach the TUI to a stored session
//!   mypi sessions             list sessions (script-friendly; goes through
//!                             the daemon's socket, never touches the db)
//!   mypi replay <id> <round>  rebuild one stored round's request (audit)
//!
//! Flags: --help/-h, --version/-V, --resume, --model <spec>.

/// Parsed command line.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Cli {
    /// Run the daemon only (--server).
    pub server: bool,
    /// Attach to this session id (`attach <id>`).
    pub attach: Option<i64>,
    /// One-shot `sessions` listing (no TUI).
    pub sessions: bool,
    /// One-shot `replay <id> <round>` (no TUI).
    pub replay: Option<(i64, i64)>,
    /// Open the resume picker immediately (--resume).
    pub resume: bool,
    /// Model override for this session (`provider:id`).
    pub model: Option<String>,
}

const USAGE: &str = "\
mypi — coding agent (daemon + TUI front end)

Usage:
  mypi [options]             new session in the TUI
  mypi attach <id>           attach the TUI to a stored session
  mypi sessions              list sessions (through the daemon socket)
  mypi replay <id> <round>   rebuild a stored round's request (read-only)
  mypi --server              run the daemon in the foreground

Options:
  -h, --help        Print this help and exit
  -V, --version     Print version and exit
  --resume          Open the session-resume picker on startup
  --model <p:id>    Session model override (provider:id, not persisted)

Configuration: ~/.config/mypi/config.yaml (see models_example.yml in the repo).
Sessions:      ~/.local/share/mypi/sessions.db
Daemon socket: $XDG_RUNTIME_DIR/mypi.sock (auto-started by the TUI)";

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
                "sessions" => cli.sessions = true,
                "replay" => {
                    let v = it.next().ok_or_else(|| {
                        anyhow::anyhow!("replay needs: mypi replay <session id> <round>")
                    })?;
                    let id: i64 = v
                        .parse()
                        .map_err(|_| anyhow::anyhow!("replay expects a numeric session id, got `{v}`"))?;
                    let v = it.next().ok_or_else(|| {
                        anyhow::anyhow!("replay needs a round number: mypi replay <id> <round>")
                    })?;
                    let round: i64 = v
                        .parse()
                        .map_err(|_| anyhow::anyhow!("replay expects a numeric round, got `{v}`"))?;
                    cli.replay = Some((id, round));
                }
                other => anyhow::bail!("unknown argument `{other}` — see `mypi --help`"),
            }
        }
        Ok(cli)
    }

    /// True when this invocation is a one-shot query (no TUI, no daemon
    /// spawn unless one is needed to answer).
    pub fn is_oneshot(&self) -> bool {
        self.server || self.sessions || self.replay.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> anyhow::Result<Cli> {
        Cli::parse(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn empty_args_are_defaults() {
        assert_eq!(parse(&[]).unwrap(), Cli::default());
    }

    #[test]
    fn resume_flag() {
        assert!(parse(&["--resume"]).unwrap().resume);
    }

    #[test]
    fn model_requires_provider_prefix() {
        let c = parse(&["--model", "local:gpt-x"]).unwrap();
        assert_eq!(c.model.as_deref(), Some("local:gpt-x"));
        // Bare id is rejected with guidance.
        assert!(parse(&["--model", "gpt-x"]).is_err());
        // Missing value is rejected.
        assert!(parse(&["--model"]).is_err());
    }

    #[test]
    fn unknown_arg_is_error() {
        assert!(parse(&["--resune"]).is_err());
        assert!(parse(&["extra-positional"]).is_err());
    }

    #[test]
    fn server_flag() {
        assert!(parse(&["--server"]).unwrap().server);
        assert!(parse(&["--server"]).unwrap().is_oneshot());
        assert!(!parse(&[]).unwrap().is_oneshot());
    }

    #[test]
    fn attach_takes_a_numeric_id() {
        let c = parse(&["attach", "42"]).unwrap();
        assert_eq!(c.attach, Some(42));
        assert!(!c.is_oneshot(), "attach is a TUI mode, not a one-shot");
        assert!(parse(&["attach"]).is_err(), "missing id rejected");
        assert!(parse(&["attach", "abc"]).is_err(), "non-numeric rejected");
    }

    #[test]
    fn sessions_is_a_oneshot() {
        assert!(parse(&["sessions"]).unwrap().sessions);
        assert!(parse(&["sessions"]).unwrap().is_oneshot());
    }

    #[test]
    fn replay_takes_two_numbers() {
        let c = parse(&["replay", "7", "3"]).unwrap();
        assert_eq!(c.replay, Some((7, 3)));
        assert!(c.is_oneshot());
        assert!(parse(&["replay", "7"]).is_err(), "missing round rejected");
        assert!(parse(&["replay", "a", "3"]).is_err());
    }
}
