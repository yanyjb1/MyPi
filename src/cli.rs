//! CLI argument parsing — hand-rolled, no clap (dep policy).
//!
//! Supported flags:
//!   --help / -h        usage text, exit 0
//!   --version / -V     version string, exit 0
//!   --resume           open the session-resume picker before the first frame
//!   --model <spec>     override the default model for this launch
//!                      (`<provider>:<id>`; not persisted — /model does that)
//!
//! Anything else is a hard error (no positional args by design: the TUI
//! is the only surface).

/// Parsed command line.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Cli {
    /// Open the resume picker immediately (--resume).
    pub resume: bool,
    /// Model override for this session (`provider:id`).
    pub model: Option<String>,
}

const USAGE: &str = "\
mypi — TUI coding agent

Usage: mypi [options]

Options:
  -h, --help        Print this help and exit
  -V, --version     Print version and exit
  --resume          Open the session-resume picker on startup
  --model <p:id>    Session model override (provider:id, not persisted)

Configuration: ~/.config/mypi/config.yaml (see models_example.yml in the repo).
Sessions:      ~/.local/share/mypi/sessions.db";

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
                "--resume" => cli.resume = true,
                "--model" => {
                    let v = it.next().ok_or_else(|| anyhow::anyhow!("--model needs a value: --model <provider:id>"))?;
                    if !v.contains(':') {
                        anyhow::bail!("--model expects <provider>:<id>, got `{v}`");
                    }
                    cli.model = Some(v);
                }
                other => anyhow::bail!(
                    "unknown argument `{other}` — see `mypi --help`"
                ),
            }
        }
        Ok(cli)
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
}
