//! Profiles — swappable system prompts, plus a per-profile compaction instruction and tool
//! roster. A profile is a directory under `profiles/`; its dotted relative path is its name.

use anyhow::{Context as _, bail};
use std::path::{Path, PathBuf};

use crate::server::config::Config;

/// The built-in `oh-my-pi`: omp's system prompt minus the tool-list sections. It depends on no
/// file.
pub const OMP_PROFILE: &str = "oh-my-pi";

/// The system prompt of a built-in profile, by name.
fn builtin_system(name: &str) -> Option<&'static str> {
    match name {
        OMP_PROFILE => Some(crate::server::config::prompts::omp_system()),
        "default" => Some(crate::server::config::prompts::builtin_system()),
        _ => None,
    }
}

/// One discovered profile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    /// Dot-joined relative name (`default`, `novelist`, `work.rust`).
    pub name: String,
    /// Absolute path of `system.md`.
    system_path: PathBuf,
    /// Absolute path of `tools.yaml` (absent → all tools enabled).
    tools_path: Option<PathBuf>,
    /// Absolute path of `compact.md` (absent → the config's
    /// `compact.instruction_file`, else the built-in instruction).
    compact_path: Option<PathBuf>,
}

impl Profile {
    pub fn load_system(&self) -> anyhow::Result<String> {
        std::fs::read_to_string(&self.system_path)
            .with_context(|| format!("failed to read the profile: {}", self.system_path.display()))
    }

    /// The profile's compaction instruction. `None` = this profile does not
    pub fn load_compact(&self) -> anyhow::Result<Option<String>> {
        let Some(p) = &self.compact_path else {
            return Ok(None);
        };
        let text = std::fs::read_to_string(p)
            .with_context(|| format!("failed to read compact.md: {}", p.display()))?;
        Ok(Some(text))
    }

    /// Enabled tool names; `None` = no restriction (all tools).
    pub fn load_tools(&self) -> anyhow::Result<Option<Vec<String>>> {
        let Some(p) = &self.tools_path else {
            return Ok(None);
        };
        let text = std::fs::read_to_string(p)
            .with_context(|| format!("failed to read tools.yaml: {}", p.display()))?;
        let v: serde_yaml::Value = serde_yaml::from_str(&text)
            .with_context(|| format!("invalid YAML: {}", p.display()))?;
        let Some(list) = v.get("enabled").and_then(|e| e.as_sequence()) else {
            bail!("tools.yaml needs an `enabled:` list ({})", p.display());
        };
        let names = list
            .iter()
            .filter_map(|i| i.as_str().map(str::to_string))
            .collect::<Vec<_>>();
        if names.is_empty() {
            bail!(
                "tools.yaml has an empty enabled list — keep at least one tool, or drop the file"
            );
        }
        Ok(Some(names))
    }
}

/// `profiles/<name>` walk: relative dir path → dotted profile name.
fn dir_name(rel: &Path) -> String {
    rel.components()
        .map(|c| c.as_os_str().to_string_lossy().to_string())
        .collect::<Vec<_>>()
        .join(".")
}

/// Discover all profiles under `base`. Returns them sorted by name.
/// A nested layout `profiles/work/rust/system.md` becomes `work.rust`.
pub fn discover(base: &Path) -> Vec<Profile> {
    let mut out = Vec::new();
    let Ok(rd) = std::fs::read_dir(base) else {
        return out;
    };
    walk(rd, PathBuf::from(base), &mut out);
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn walk(rd: std::fs::ReadDir, root: PathBuf, out: &mut Vec<Profile>) {
    for entry in rd.flatten() {
        let p = entry.path();
        if !p.is_dir() {
            continue;
        }
        let system = p.join("system.md");
        if system.is_file() {
            let rel = p.strip_prefix(&root).unwrap_or(&p).to_path_buf();
            out.push(Profile {
                name: dir_name(&rel),
                system_path: system,
                tools_path: {
                    let t = p.join("tools.yaml");
                    t.is_file().then_some(t)
                },
                compact_path: {
                    let c = p.join("compact.md");
                    c.is_file().then_some(c)
                },
            });
        }
        // Recurse regardless: sub-dirs may hold deeper profiles.
        if let Ok(sub) = std::fs::read_dir(&p) {
            walk(sub, root.clone(), out);
        }
    }
}

/// The active profile: config's `profile:` key, or the built-in `oh-my-pi`.
pub fn active_name(cfg: &Config) -> String {
    cfg.app
        .server
        .agent
        .profile
        .clone()
        .unwrap_or_else(|| OMP_PROFILE.into())
}

/// A profile resolved to concrete values: the system prompt, the tool roster,
/// and the compaction instruction to use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProfile {
    pub system: String,
    /// Enabled tool names; `None` = no restriction (all tools).
    pub tools: Option<Vec<String>>,
    /// The instruction text, already read. Precedence: the profile's own
    /// `compact.md`, else the file named in `config.yaml`, else the built-in.
    pub compact_instruction: String,
}

/// Read the compaction instruction; a caller gets the text, not a path. A named file that cannot
/// be read is a hard error.
fn instruction_text(profile: Option<&PathBuf>, cfg: &Config) -> anyhow::Result<String> {
    if let Some(p) = profile {
        return std::fs::read_to_string(p)
            .with_context(|| format!("failed to read the profile's compact.md: {}", p.display()));
    }
    if let Some(p) = cfg.app.server.request.compact.instruction_file.as_deref() {
        if let Ok(text) = std::fs::read_to_string(p) {
            return Ok(text);
        }
    }
    Ok(crate::server::config::prompts::compact_instruction().into())
}

/// Project first, global as the fallback, and a duplicate name is decided by the earlier one.
/// A union, not a replacement.
fn discovered(dirs: &[(crate::server::config::paths::Origin, PathBuf)]) -> Vec<Profile> {
    let mut out: Vec<Profile> = Vec::new();
    for (_, dir) in dirs {
        for p in discover(dir) {
            if out.iter().any(|seen| seen.name == p.name) {
                continue;
            }
            out.push(p);
        }
    }
    out
}

/// Resolve the active profile to concrete values.
pub fn resolve(cfg: &Config, name: &str) -> anyhow::Result<ResolvedProfile> {
    let dirs = cfg.paths.profiles_dirs();
    let found = discovered(&dirs);
    // The built-in identity comes first; a directory of the same name under `profiles/` is the
    // user's explicit override.
    if let Some(builtin) = builtin_system(name)
        && !found.iter().any(|p| p.name == name)
    {
        return Ok(ResolvedProfile {
            system: builtin.into(),
            tools: None,
            compact_instruction: instruction_text(None, cfg)?,
        });
    }
    if !found.is_empty() || !dirs.is_empty() {
        if let Some(p) = found.iter().find(|p| p.name == name) {
            return Ok(ResolvedProfile {
                system: p.load_system()?,
                tools: p.load_tools()?,
                compact_instruction: instruction_text(p.compact_path.as_ref(), cfg)?,
            });
        }
        if let Some(builtin) = builtin_system(name) {
            return Ok(ResolvedProfile {
                system: builtin.into(),
                tools: None,
                compact_instruction: instruction_text(None, cfg)?,
            });
        }
        let names = found.iter().map(|p| p.name.as_str()).collect::<Vec<_>>();
        bail!("unknown profile: {name}. Available: {}", names.join(", "));
    }
    // No profiles dir at all: only the built-in identity exists.
    Ok(ResolvedProfile {
        system: crate::server::config::prompts::builtin_system().into(),
        tools: None,
        compact_instruction: instruction_text(None, cfg)?,
    })
}

/// List available profile names (always includes `default`).
pub fn list(cfg: &Config) -> Vec<String> {
    let mut names = discovered(&cfg.paths.profiles_dirs())
        .into_iter()
        .map(|p| p.name)
        .collect::<Vec<_>>();
    if !names.iter().any(|n| n == "default") {
        names.insert(0, "default".into());
    }
    if !names.iter().any(|n| n == OMP_PROFILE) {
        names.push(OMP_PROFILE.into());
    }
    names
}
