// The session's shell: which program runs a command line, and how a stored text reads as a file.

use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Result, bail};

use super::os::{self, Family};

// The shell family a command line is written for.
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

    // A family runs only the shells it owns.
    fn available(self) -> bool {
        match os::current().family {
            Family::Unix => matches!(self, Kind::Bash | Kind::Zsh),
            Family::Windows => matches!(self, Kind::Pwsh | Kind::Niu),
        }
    }
}

// Every shell this binary accepts, in the order configuration lists them.
pub fn names() -> Vec<&'static str> {
    [Kind::Bash, Kind::Zsh, Kind::Pwsh, Kind::Niu]
        .into_iter()
        .filter(|k| k.available())
        .map(Kind::name)
        .collect()
}

// One shell backend: the program, plus the shape one command line takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shell {
    kind: Kind,
    program: PathBuf,
}

impl Shell {
    // This platform's probe order: `$SHELL`, bash, zsh on unix; pwsh, niu on windows.
    pub fn detect() -> Result<Self> {
        match os::current().family {
            Family::Unix => detect_unix(),
            Family::Windows => detect_windows(),
        }
    }

    // A configured name -> the backend; only a concrete name is accepted.
    pub fn resolve(name: &str) -> Result<Self> {
        let name = name.trim();
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
        let Some(program) = find_on_path(kind.name()) else {
            bail!(
                "no `{name}` on PATH; install it, or name another backend in config.server.agent.tools.shell"
            );
        };
        Ok(Self { kind, program })
    }

    pub fn kind(&self) -> Kind {
        self.kind
    }

    pub fn name(&self) -> &'static str {
        self.kind.name()
    }

    // The unix shells are the guarded ones; `pwsh` and `niu` run the line as written.
    pub fn guarded(&self) -> bool {
        matches!(self.kind, Kind::Bash | Kind::Zsh)
    }

    // One command line, ready to spawn: the program, its one-shot shape, and how the backend emits UTF-8.
    pub fn command(&self, line: &str) -> Command {
        let mut cmd = Command::new(&self.program);
        match self.kind {
            Kind::Bash | Kind::Zsh => {
                cmd.arg("-c").arg(line);
                cmd.env("LC_CTYPE", utf8_locale());
            }
            Kind::Niu => {
                cmd.arg("-c").arg(line);
            }
            Kind::Pwsh => {
                cmd.args(["-NoProfile", "-NonInteractive", "-Command"])
                    .arg(pwsh_line(line));
            }
        }
        cmd
    }

    // The single shell token naming one stored file, quoted so the path survives as one word.
    pub fn path_token(&self, path: &Path) -> String {
        let text = path.display().to_string();
        match self.kind {
            Kind::Bash | Kind::Zsh | Kind::Niu => posix(&text),
            Kind::Pwsh => pwsh_quote(&text),
        }
    }
}

// The UTF-8 character set forced on unix: Linux has C.UTF-8, macOS accepts UTF-8.
fn utf8_locale() -> &'static str {
    match os::current().name {
        "macos" => "UTF-8",
        _ => "C.UTF-8",
    }
}

// Ask pwsh for UTF-8 on the pipe first; a failed setting never refuses the command.
fn pwsh_line(line: &str) -> String {
    format!("try {{ [Console]::OutputEncoding = [Text.Encoding]::UTF8 }} catch {{}}; {line}")
}

// Probe unix in order, using `$SHELL` when it names a shell we run, then PATH.
fn detect_unix() -> Result<Shell> {
    let preferred = std::env::var("SHELL").ok().and_then(|raw| {
        let path = PathBuf::from(&raw);
        let kind = match path.file_name()?.to_str()? {
            "bash" => Kind::Bash,
            "zsh" => Kind::Zsh,
            _ => return None,
        };
        let program = if path.is_absolute() {
            is_executable(&path).then_some(path)?
        } else {
            find_on_path(kind.name())?
        };
        Some((kind, program))
    });
    let found = preferred.or_else(|| {
        [Kind::Bash, Kind::Zsh]
            .into_iter()
            .find_map(|kind| find_on_path(kind.name()).map(|program| (kind, program)))
    });
    found
        .map(|(kind, program)| Shell { kind, program })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no shell found: $SHELL does not name bash or zsh, and neither is executable; install one of {} and name it in config.server.agent.tools.shell",
                names().join(", ")
            )
        })
}

// Probe windows in order: pwsh, then niu.
fn detect_windows() -> Result<Shell> {
    [Kind::Pwsh, Kind::Niu]
        .into_iter()
        .find_map(|kind| find_on_path(kind.name()).map(|program| (kind, program)))
        .map(|(kind, program)| Shell { kind, program })
        .ok_or_else(|| {
            anyhow::anyhow!(
                "no shell found: pwsh.exe and niu.exe are not on PATH; install one of {} and name it in config.server.agent.tools.shell",
                names().join(", ")
            )
        })
}

// PATH lookup only: a shell is never run to ask its version.
fn find_on_path(program: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).find_map(|dir| {
        let mut candidates = vec![dir.join(program)];
        if cfg!(windows) {
            candidates.insert(0, dir.join(format!("{program}.exe")));
        }
        candidates
            .into_iter()
            .find(|candidate| is_executable(candidate))
    })
}

// A file the current user can run: the executable bit on unix, an existing file on windows.
fn is_executable(path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        meta.permissions().mode() & 0o111 != 0
    }
    #[cfg(windows)]
    {
        true
    }
}

// Single-quoted for a POSIX shell; a quote closes, escapes and reopens.
fn posix(text: &str) -> String {
    format!("'{}'", text.replace('\'', r"'\''"))
}

// Single-quoted for PowerShell; a quote doubles.
fn pwsh_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "''"))
}
