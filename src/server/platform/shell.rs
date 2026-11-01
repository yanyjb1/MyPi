//! The session's shell: which program runs a command line, and how a stored text reads as a file.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Result, bail};

use super::os::{self, Family};

/// The shell family a command line is written for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Bash,
    Zsh,
    Pwsh,
    Niu,
}

impl Kind {
    pub fn name(self) -> &'static str {
        match self {
            Kind::Bash => "bash",
            Kind::Zsh => "zsh",
            Kind::Pwsh => "pwsh",
            Kind::Niu => "niu",
        }
    }

    /// A family runs only the shells it owns.
    fn available(self) -> bool {
        match os::current().family {
            Family::Unix => matches!(self, Kind::Bash | Kind::Zsh),
            Family::Windows => matches!(self, Kind::Pwsh | Kind::Niu),
        }
    }
}

/// Every shell this binary accepts, in the order configuration lists them.
pub fn names() -> Vec<&'static str> {
    [Kind::Bash, Kind::Zsh, Kind::Pwsh, Kind::Niu]
        .into_iter()
        .filter(|k| k.available())
        .map(Kind::name)
        .collect()
}

/// One shell backend: the program, plus the shape one command line takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shell {
    kind: Kind,
    program: PathBuf,
}

impl Shell {
    /// This platform's default: `bash` on unix, `pwsh` on windows.
    pub fn default() -> Self {
        let kind = match os::current().family {
            Family::Unix => Kind::Bash,
            Family::Windows => Kind::Pwsh,
        };
        Self {
            kind,
            program: PathBuf::from(kind.name()),
        }
    }

    /// A configured name → the backend; a shell the platform does not run is refused here.
    pub fn resolve(name: Option<&str>) -> Result<Self> {
        let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) else {
            return Ok(Self::default());
        };
        let kind = match name {
            "bash" => Kind::Bash,
            "zsh" => Kind::Zsh,
            "pwsh" => Kind::Pwsh,
            "niu" => Kind::Niu,
            other => bail!(
                "unknown shell `{other}`; this build knows {}",
                names().join(", ")
            ),
        };
        if !kind.available() {
            bail!(
                "`{name}` does not run on {}; this platform offers {}",
                os::current().name,
                names().join(", ")
            );
        }
        Ok(Self {
            kind,
            program: PathBuf::from(kind.name()),
        })
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn name(&self) -> &'static str {
        self.kind.name()
    }

    /// The unix shells are the guarded ones; `pwsh` and `niu` run the line as written.
    pub fn guarded(&self) -> bool {
        matches!(self.kind, Kind::Bash | Kind::Zsh)
    }

    /// One command line, ready to spawn: the program plus its own one-shot argument shape.
    pub fn command(&self, line: &str) -> Command {
        let mut cmd = Command::new(&self.program);
        match self.kind {
            Kind::Bash | Kind::Zsh | Kind::Niu => {
                cmd.arg("-c");
            }
            Kind::Pwsh => {
                cmd.args(["-NoProfile", "-NonInteractive", "-Command"]);
            }
        }
        cmd.arg(line);
        cmd
    }

    /// The token that prints one stored file, in this shell's own words.
    pub fn read_file(&self, path: &Path) -> String {
        let text = path.display().to_string();
        match self.kind {
            Kind::Bash | Kind::Zsh | Kind::Niu => format!("cat {}", posix(&text)),
            Kind::Pwsh => format!("Get-Content -Raw -LiteralPath {}", pwsh_quote(&text)),
        }
    }
}

/// Single-quoted for a POSIX shell; a quote closes, escapes and reopens.
fn posix(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

/// Single-quoted for PowerShell; a quote doubles.
fn pwsh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}
