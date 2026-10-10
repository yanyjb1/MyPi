//! Zones and paths: what the file tools, `bash` and `cd` may touch. One criterion: the session
//! workspace plus the system temp dir (rationale in `src/server/agent/README.md`).

use anyhow::{Result, anyhow};

/// Where the session stands, plus the directory it started in. `cwd` moves, `root` does not.
pub struct Workspace {
    cwd: std::path::PathBuf,
    root: std::path::PathBuf,
}

impl Workspace {
    pub fn new(cwd: impl Into<std::path::PathBuf>) -> Self {
        let cwd = cwd.into();
        Self {
            root: cwd.clone(),
            cwd,
        }
    }

    pub fn root(&self) -> &std::path::Path {
        &self.root
    }

    pub fn cwd(&self) -> &std::path::Path {
        &self.cwd
    }

    /// The licensed zones: **where the session stands**, plus temp. Hangs off `cwd`, not the start
    /// directory, so a deliberate `cd` carries the licence with it.
    pub fn zones(&self) -> Vec<std::path::PathBuf> {
        licensed_zones(&self.cwd)
    }

    /// Resolve a model-given path: relative to `cwd`, `..` and symlinks normalized first, and the
    /// result MUST land inside a licensed zone.
    pub fn resolve(&self, raw: &str, must_exist: bool) -> Result<std::path::PathBuf> {
        resolve_path(&self.cwd, &self.zones(), raw, must_exist)
    }

    /// Land on a new working directory. The caller checks where it may land (see `cd`).
    pub fn chdir(&mut self, real: std::path::PathBuf) {
        self.cwd = real;
    }
}

/// Canonicalize as far as it goes, then re-attach the tail. The `write` target may not exist yet,
/// and the zone check MUST run on the **real** path.
pub(crate) fn canonicalize_lenient(p: &std::path::Path) -> std::path::PathBuf {
    let mut tail: Vec<std::ffi::OsString> = Vec::new();
    let mut cur = p.to_path_buf();
    loop {
        if let Ok(real) = cur.canonicalize() {
            let mut out = real;
            for part in tail.iter().rev() {
                out.push(part);
            }
            return out;
        }
        match (cur.parent().map(|x| x.to_path_buf()), cur.file_name()) {
            (Some(parent), Some(name)) => {
                tail.push(name.to_os_string());
                cur = parent;
            }
            _ => return p.to_path_buf(),
        }
    }
}

/// The zones **file tools** may touch: the session workspace plus the system temp dir. `bash`'s
/// delete rule reads the same two, so "inside" has one definition.
pub fn licensed_zones(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut zones = vec![canonicalize_lenient(root)];
    let tmp = std::env::temp_dir();
    zones.push(canonicalize_lenient(&tmp));
    zones.dedup();
    zones
}

/// Resolve a model-given path; refuse when it lands outside every licensed zone.
fn resolve_path(
    cwd: &std::path::Path,
    zones: &[std::path::PathBuf],
    raw: &str,
    must_exist: bool,
) -> Result<std::path::PathBuf> {
    anyhow::ensure!(!raw.trim().is_empty(), "empty path");
    let p = std::path::Path::new(raw);
    let p = if p.is_absolute() {
        p.to_path_buf()
    } else {
        cwd.join(p)
    };
    let real = canonicalize_lenient(&p);
    if !zones.iter().any(|z| real.starts_with(z)) {
        anyhow::bail!(
            "denied: {} is outside the workspace ({}) and the temp dir — \
             file tools read and write only those two places",
            real.display(),
            zones
                .first()
                .map(|z| z.display().to_string())
                .unwrap_or_default()
        );
    }
    if must_exist {
        if !real.exists() {
            return Err(anyhow!("file not found: {}", real.display()));
        }
        if real.is_dir() {
            return Err(anyhow!(
                "{} is a directory; only files can be read or edited",
                real.display()
            ));
        }
    }
    Ok(real)
}
