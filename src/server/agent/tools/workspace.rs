//! Zones and paths: what the file tools, `bash` and `cd` may touch. One criterion: the session
//! workspace plus the system temp dir (rationale in `src/server/agent/README.md`).

use anyhow::{Context as _, Result, anyhow};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, RwLock};

/// Where the session stands. The handle is the one source: every read takes it, `chdir` writes it.
pub struct Workspace {
    shared: super::contract::call::Cwd,
}

impl Workspace {
    pub fn new(shared: super::contract::call::Cwd) -> Self {
        Self { shared }
    }

    /// The session's own directory handle, for the session that builds this.
    pub fn shared(&self) -> super::contract::call::Cwd {
        self.shared.clone()
    }

    /// The directory as it stands right now.
    pub fn cwd(&self) -> std::path::PathBuf {
        self.shared
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    /// The licensed zones: **where the session stands**, plus temp. Hangs off `cwd`, not the start
    /// directory, so a deliberate `cd` carries the licence with it.
    pub fn zones(&self) -> Vec<std::path::PathBuf> {
        licensed_zones(&self.cwd())
    }

    /// Resolve a model-given path: relative to `cwd`, `..` and symlinks normalized first, and the
    /// result MUST land inside a licensed zone.
    pub fn resolve(&self, raw: &str, must_exist: bool) -> Result<std::path::PathBuf> {
        resolve_path(&self.cwd(), &self.zones(), raw, must_exist)
    }

    /// Land on a new working directory. The caller checks where it may land (see `cd`).
    pub fn chdir(&self, real: std::path::PathBuf) {
        *self.shared.write().unwrap_or_else(|e| e.into_inner()) = real;
    }

    /// Run a read-modify-write under the path's exclusive lock.
    pub fn with_write<T>(
        &self,
        path: &std::path::Path,
        f: impl FnOnce() -> Result<T>,
    ) -> Result<T> {
        let lock = path_lock(path);
        let _guard = lock.write().unwrap_or_else(|e| e.into_inner());
        f()
    }
}

static LOCKS: LazyLock<Mutex<HashMap<std::path::PathBuf, Arc<RwLock<()>>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn path_lock(path: &std::path::Path) -> Arc<RwLock<()>> {
    let mut map = LOCKS.lock().unwrap_or_else(|e| e.into_inner());
    map.entry(path.to_path_buf())
        .or_insert_with(|| Arc::new(RwLock::new(())))
        .clone()
}

static NEXT_TMP: AtomicU64 = AtomicU64::new(0);

/// Write through a sibling temp file, then rename over the target.
pub fn write_atomic(path: &std::path::Path, content: &str) -> Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("no parent directory for {}", path.display()))?;
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
    let tmp = parent.join(format!(
        ".{name}.tmp-{}-{}",
        std::process::id(),
        NEXT_TMP.fetch_add(1, Ordering::Relaxed)
    ));
    let result = std::fs::write(&tmp, content).and_then(|_| std::fs::rename(&tmp, path));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result.map_err(|e| anyhow!("failed to write {}: {e}", path.display()))
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

/// A directory the session may stand in: it exists, it is a directory, and
/// [`may_stand_in`] allows it.
pub fn resolve_cwd(cwd: &std::path::Path, raw: &str) -> Result<std::path::PathBuf> {
    let real = resolve_dir(cwd, raw)?;
    may_stand_in(&real)?;
    Ok(real)
}

/// A directory that exists and is a directory; where it may stand is another question.
pub fn resolve_dir(cwd: &std::path::Path, raw: &str) -> Result<std::path::PathBuf> {
    anyhow::ensure!(!raw.trim().is_empty(), "empty path");
    let raw = raw.trim();
    let home = crate::server::platform::env::home();
    let target = match raw.strip_prefix('~') {
        Some(rest) => home
            .as_ref()
            .context("no home directory to expand `~` into")?
            .join(rest.trim_start_matches('/')),
        None => std::path::PathBuf::from(raw),
    };
    let target = if target.is_absolute() {
        target
    } else {
        cwd.join(target)
    };
    let real = target
        .canonicalize()
        .with_context(|| format!("no such dir: {raw}"))?;
    anyhow::ensure!(real.is_dir(), "not a directory: {raw}");
    Ok(real)
}

/// The launch directory `init` was handed: it exists, so only where it may stand is asked, and a
/// directory it may not stand in becomes the temp dir.
pub fn launch_dir(raw: &str) -> std::path::PathBuf {
    let given = std::path::Path::new(raw);
    let given = if given.is_absolute() {
        given.to_path_buf()
    } else {
        std::env::current_dir().unwrap_or_default().join(given)
    };
    let real = canonicalize_lenient(&given);
    match may_stand_in(&real) {
        Ok(()) => real,
        Err(_) => canonicalize_lenient(&crate::server::platform::env::temp_dir()),
    }
}

/// Where a session may stand: not the filesystem root, not a top-level directory (the temp dir
/// and everything under it excepted), and not a home directory itself.
pub fn may_stand_in(real: &std::path::Path) -> Result<()> {
    anyhow::ensure!(!real.as_os_str().is_empty(), "denied: that is no directory");
    let temp = canonicalize_lenient(&crate::server::platform::env::temp_dir());
    if !temp.as_os_str().is_empty() && real.starts_with(&temp) {
        return Ok(());
    }
    if let Some(home) = crate::server::platform::env::home().map(|home| canonicalize_lenient(&home))
        && !home.as_os_str().is_empty()
    {
        anyhow::ensure!(
            real != home,
            "denied: the home directory itself is not a workspace"
        );
        // Every home directory under the parent is barred; work below one is fair ground.
        if let Some(tree) = home.parent()
            && tree.parent().is_some()
            && real.starts_with(tree)
            && real.components().count() <= tree.components().count() + 1
        {
            anyhow::bail!(
                "denied: {} is a home directory; stand inside one instead",
                real.display()
            );
        }
    }
    anyhow::ensure!(
        real.components().count() >= 3,
        "denied: {} is a top-level directory; stand in one of its children",
        real.display()
    );
    Ok(())
}

/// The zones **file tools** may touch: the session workspace plus the system temp dir. `bash`'s
/// delete rule reads the same two, so "inside" has one definition.
pub fn licensed_zones(root: &std::path::Path) -> Vec<std::path::PathBuf> {
    let mut zones = vec![canonicalize_lenient(root)];
    let tmp = crate::server::platform::env::temp_dir();
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
    if !zones
        .iter()
        .any(|z| !z.as_os_str().is_empty() && real.starts_with(z))
    {
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
