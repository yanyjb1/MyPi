//! Where the config sources live.

use std::path::{Path, PathBuf};

pub const CONFIG_FILE: &str = "config.yaml";
pub const MODELS_FILE: &str = "models.yml";
pub const PROFILES_SUBDIR: &str = "profiles";
pub const PROFILE_CONFIG: &str = "config.yaml";
pub const SYSTEM_FILE: &str = "system.md";
pub const COMPACT_FILE: &str = "compact.md";
pub const TOOLS_FILE: &str = "tools.yaml";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Project,
    Global,
}

#[derive(Debug, Clone)]
pub struct ConfigPaths {
    roots: Vec<(Origin, PathBuf)>,
}

impl ConfigPaths {
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

    pub fn from_env() -> Self {
        if let Some(dir) = env_root() {
            return Self::new(vec![(Origin::Global, dir)]);
        }
        let mut roots = Vec::new();
        if let Ok(cwd) = std::env::current_dir() {
            roots.push((Origin::Project, cwd));
        }
        roots.push((Origin::Global, crate::server::platform::env::config_dir()));
        Self::new(roots)
    }

    pub fn global_root(&self) -> Option<&Path> {
        self.roots
            .iter()
            .find(|(o, _)| *o == Origin::Global)
            .map(|(_, p)| p.as_path())
    }

    pub fn config_files(&self) -> Vec<(Origin, PathBuf)> {
        self.existing(CONFIG_FILE, false)
    }

    pub fn models_file(&self) -> Option<PathBuf> {
        let root = self.global_root()?;
        let path = root.join(MODELS_FILE);
        path.is_file().then_some(path)
    }

    pub fn profiles_dirs(&self) -> Vec<(Origin, PathBuf)> {
        self.existing(PROFILES_SUBDIR, true)
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

fn env_root() -> Option<PathBuf> {
    crate::server::platform::env::var_os("MYPI_CONFIG_DIR").map(PathBuf::from)
}

fn absolute(p: &Path) -> PathBuf {
    if p.is_absolute() {
        return p.to_path_buf();
    }
    match std::env::current_dir() {
        Ok(cwd) => cwd.join(p),
        Err(_) => p.to_path_buf(),
    }
}
