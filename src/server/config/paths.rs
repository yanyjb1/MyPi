//! Where the config files live — **only** this question is answered here. Two roots (project,
//! then global), each resident decided on its own; details in this folder's `README.md`.

use std::path::{Path, PathBuf};

/// The residents' file names. Adding a resident means adding it here.
pub const CONFIG_FILE: &str = "config.yaml";
pub const MODELS_FILE: &str = "models.yml";
pub const PROFILES_SUBDIR: &str = "profiles";
pub const THEMES_SUBDIR: &str = "themes";

/// Where one candidate path came from. Callers tell "the project's" from "the
/// global one" by this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    /// The project root (the startup directory, or the one `MYPI_CONFIG_DIR`
    /// names).
    Project,
    /// The global root (`$XDG_CONFIG_HOME/mypi`), the fallback copy.
    Global,
}

impl Origin {
    /// The human name used in error messages.
    pub fn label(self) -> &'static str {
        match self {
            Origin::Project => "项目",
            Origin::Global => "全局",
        }
    }
}

/// Which roots this load used, in built-in order (project first).
#[derive(Debug, Clone)]
pub struct ConfigPaths {
    roots: Vec<(Origin, PathBuf)>,
}

impl ConfigPaths {
    /// Use the given roots directly: a relative path is made absolute against the startup
    /// directory, and the same directory is kept once.
    pub fn new(roots: Vec<(Origin, PathBuf)>) -> Self {
        let mut out: Vec<(Origin, PathBuf)> = Vec::new();
        for (origin, root) in roots {
            let root = absolute(&root);
            if out.iter().any(|(_, seen)| *seen == root) {
                continue;
            }
            out.push((origin, root));
        }
        Self { roots: out }
    }

    /// The production roots: `MYPI_CONFIG_DIR` alone when it is set (as the project root), else
    /// the project root plus the global root.
    pub fn from_env() -> Self {
        if let Some(dir) = env_root() {
            return Self::new(vec![(Origin::Project, dir)]);
        }
        let mut roots = Vec::new();
        if let Ok(cwd) = std::env::current_dir() {
            roots.push((Origin::Project, cwd));
        }
        roots.push((Origin::Global, crate::server::platform::env::config_dir()));
        Self::new(roots)
    }

    /// Every candidate root, in built-in order.
    pub fn roots(&self) -> &[(Origin, PathBuf)] {
        &self.roots
    }

    /// The global root — the only one ever written to. Under `MYPI_CONFIG_DIR` there is none.
    pub fn global_root(&self) -> Option<&Path> {
        self.roots
            .iter()
            .find(|(o, _)| *o == Origin::Global)
            .map(|(_, p)| p.as_path())
    }

    /// The `config.yaml` files that exist, in root order.
    pub fn config_files(&self) -> Vec<(Origin, PathBuf)> {
        self.existing(CONFIG_FILE, false)
    }

    /// The `models.yml` files that exist, in root order.
    pub fn models_files(&self) -> Vec<(Origin, PathBuf)> {
        self.existing(MODELS_FILE, false)
    }

    /// The `profiles/` directories that exist, in root order (project first: a
    /// name present in both is decided by the project's copy).
    pub fn profiles_dirs(&self) -> Vec<(Origin, PathBuf)> {
        self.existing(PROFILES_SUBDIR, true)
    }

    /// The `themes/` directories that exist, in root order.
    pub fn themes_dirs(&self) -> Vec<(Origin, PathBuf)> {
        self.existing(THEMES_SUBDIR, true)
    }

    fn existing(&self, entry: &str, dir: bool) -> Vec<(Origin, PathBuf)> {
        self.roots
            .iter()
            .filter_map(|(origin, root)| {
                let p = root.join(entry);
                let found = if dir { p.is_dir() } else { p.is_file() };
                found.then_some((*origin, p))
            })
            .collect()
    }
}

/// `MYPI_CONFIG_DIR`: non-empty counts (an empty string means "not set").
fn env_root() -> Option<PathBuf> {
    crate::server::platform::env::var_os("MYPI_CONFIG_DIR").map(PathBuf::from)
}

/// A relative path is made absolute against the startup directory; with no
/// startup directory it is returned as it came.
fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        return p.to_path_buf();
    }
    match std::env::current_dir() {
        Ok(cwd) => cwd.join(p),
        Err(_) => p.to_path_buf(),
    }
}
