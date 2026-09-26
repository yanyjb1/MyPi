//! Profiles — swappable system prompts + per-profile tool rosters.
//!
//! Layout (mirrors the config dir, `~/.config/mypi/profiles/`):
//!
//! ```text
//! profiles/
//!   default/
//!     system.md      ← the system prompt (replaces the built-in one)
//!     tools.yaml     ← optional; enabled tool names (default: all)
//!   novelist/
//!     system.md
//! ```
//!
//! A profile switch is only legal at a **context rebuild point** —
//! session start, or right after a compaction fork — because the system
//! message is the head of the cached prefix; switching mid-session
//! would cold the KV cache and fork the model's mental state. Enforced
//! by the surface, not this module.
//!
//! Discovery is recursive (`profiles/<name>/system.md`, nested dirs
//! become dotted names); missing files degrade: a dir without
//! system.md is not a profile; no profiles dir at all = one implicit
//! built-in profile named `default` with the hard-coded prompt.

use anyhow::{Context as _, bail};
use std::path::{Path, PathBuf};

use crate::server::ai::config::Config;

/// The built-in prompt every profile replaces. Also the fallback when
/// no profiles dir exists.
pub const BUILTIN_SYSTEM: &str = "你是一个简洁的编程助手。用中文回答。";

/// 内置的 `oh-my-pi`：omp 的系统提示词（去掉工具清单那几节，见
/// [`crate::server::prompts`]）。它不依赖任何文件——没有 profiles 目录
/// 也挑得到。
pub const OMP_PROFILE: &str = "oh-my-pi";

/// 按名字找内置 profile 的系统提示词。
fn builtin_system(name: &str) -> Option<&'static str> {
    match name {
        OMP_PROFILE => Some(crate::server::prompts::OH_MY_PI_SYSTEM),
        "default" => Some(BUILTIN_SYSTEM),
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
}

impl Profile {
    pub fn load_system(&self) -> anyhow::Result<String> {
        std::fs::read_to_string(&self.system_path)
            .with_context(|| format!("读取 profile 失败：{}", self.system_path.display()))
    }

    /// Enabled tool names; `None` = no restriction (all tools).
    pub fn load_tools(&self) -> anyhow::Result<Option<Vec<String>>> {
        let Some(p) = &self.tools_path else {
            return Ok(None);
        };
        let text = std::fs::read_to_string(p)
            .with_context(|| format!("读取 tools.yaml 失败：{}", p.display()))?;
        let v: serde_yaml::Value =
            serde_yaml::from_str(&text).with_context(|| format!("无效 YAML：{}", p.display()))?;
        let Some(list) = v.get("enabled").and_then(|e| e.as_sequence()) else {
            bail!("tools.yaml 需要 `enabled:` 列表（{}）", p.display());
        };
        let names = list
            .iter()
            .filter_map(|i| i.as_str().map(str::to_string))
            .collect::<Vec<_>>();
        if names.is_empty() {
            bail!("tools.yaml 的 enabled 列表为空——至少留一个工具，否则写空 tools.yaml");
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
            });
        }
        // Recurse regardless: sub-dirs may hold deeper profiles.
        if let Ok(sub) = std::fs::read_dir(&p) {
            walk(sub, root.clone(), out);
        }
    }
}

/// The active profile: config's `profile:` key, or `default`.
pub fn active_name(cfg: &Config) -> String {
    cfg.app.profile.clone().unwrap_or_else(|| "default".into())
}

/// Resolve the active profile to a concrete (prompt, tool-filter) pair.
///
/// Resolution order: matching discovered profile → the `default`
/// profile → the built-in prompt (no dir, no restriction). Missing
/// profile *names* never fail silently here — the caller surfaces the
/// "not found" before reaching this.
pub fn resolve(cfg: &Config, name: &str) -> anyhow::Result<(String, Option<Vec<String>>)> {
    // 内置身份优先：`oh-my-pi` 是随二进制发的提示词，不靠用户的文件系统。
    // 用户若在 profiles 目录里建了同名目录，那就是他显式的**覆盖**——所以
    // 这一支只在目录里没有同名 profile 时生效。
    let base = profiles_dir(cfg)?;
    if let Some(builtin) = builtin_system(name)
        && !discover(&base).iter().any(|p| p.name == name)
    {
        return Ok((builtin.into(), None));
    }
    let found = discover(&base);
    if !found.is_empty() || base.is_dir() {
        if let Some(p) = found.iter().find(|p| p.name == name) {
            return Ok((p.load_system()?, p.load_tools()?));
        }
        if let Some(builtin) = builtin_system(name) {
            return Ok((builtin.into(), None));
        }
        let names = found.iter().map(|p| p.name.as_str()).collect::<Vec<_>>();
        bail!("未知 profile：{name}。可用：{}", names.join(", "));
    }
    // No profiles dir at all: only the built-in identity exists.
    Ok((BUILTIN_SYSTEM.into(), None))
}

/// List available profile names (always includes `default`).
pub fn list(cfg: &Config) -> Vec<String> {
    let base = profiles_dir(cfg).unwrap_or_default();
    let mut names = discover(&base)
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

/// `…/mypi/profiles` — sibling of config.yaml.
pub fn profiles_dir(_cfg: &Config) -> anyhow::Result<PathBuf> {
    Ok(Config::config_dir()?.join("profiles"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn scratch(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("mypi-profile-{}-{}", tag, std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn discovery_is_recursive_and_dotted() {
        let base = scratch("discover");
        write(&base.join("default/system.md"), "built-in");
        write(&base.join("novelist/system.md"), "小说模式");
        write(&base.join("work/rust/system.md"), "rust only");
        write(&base.join("novelist/tools.yaml"), "enabled:\n  - read\n");
        let found = discover(&base);
        let names: Vec<&str> = found.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(names, vec!["default", "novelist", "work.rust"]);
        let nov = found.iter().find(|p| p.name == "novelist").unwrap();
        assert_eq!(nov.load_tools().unwrap().unwrap(), vec!["read".to_string()]);
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 内置 profile 不依赖文件系统：`oh-my-pi` 随二进制发出去，
    /// 用户没建 profiles 目录时它照样在花名册里、照样解析得出来。
    #[test]
    fn builtin_profiles_exist_without_any_directory() {
        let omp = builtin_system(OMP_PROFILE).expect("oh-my-pi 是内置的");
        assert!(omp.contains("§ Role"), "抄的是 omp 那份");
        assert!(omp.contains("§ Delivery"), "交付契约那几节要在");
        assert!(
            !omp.contains("{{"),
            "模板标记一个都不许留（抄的是渲染后的成品）"
        );
        assert!(builtin_system("default").unwrap().contains("编程助手"));
        assert!(builtin_system("没有这个").is_none());
    }

    #[test]
    fn dir_without_system_md_is_not_a_profile() {
        let base = scratch("empty");
        std::fs::create_dir_all(base.join("hollow")).unwrap();
        assert!(discover(&base).is_empty());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn empty_tools_list_is_rejected() {
        let base = scratch("emptytools");
        write(&base.join("p/system.md"), "x");
        write(&base.join("p/tools.yaml"), "enabled: []\n");
        let found = discover(&base);
        assert!(found[0].load_tools().is_err());
        let _ = std::fs::remove_dir_all(&base);
    }
    #[test]
    fn tool_gate_hides_and_refuses() {
        // Gate one: the roster the model sees drops disabled tools but
        // always keeps `context` (the compressed-session escape hatch).
        let mut t = crate::server::agent::tools::BuiltinTools::new(std::env::temp_dir())
            .with_enabled(Some(vec!["read".into()]));
        let roster = t.definitions_for();
        let names: Vec<&str> = roster.iter().map(|d| d.function.name.as_str()).collect();
        assert!(names.contains(&"read") && names.contains(&"context"));
        assert!(!names.contains(&"bash") && !names.contains(&"browser"));

        // Gate two: execute refuses a hallucinated call.
        let call = crate::server::ai::types::ToolCall::new("c1", "bash", "{}");
        let err = format!(
            "{:#}",
            crate::server::agent::loop_rs::ToolExecutor::execute(&mut t, &call, &mut |_| {}).unwrap_err()
        );
        assert!(err.contains("disabled by the active profile"), "{err}");

        // `context` itself passes the gate despite not being listed.
        let call = crate::server::ai::types::ToolCall::new(
            "c2",
            "context",
            r##"{"intent":"查历史","anchor":"#1"}"##,
        );
        // The history is empty, so the lookup fails — but with a
        // *lookup* error, not the gate error.
        let err = format!(
            "{:#}",
            crate::server::agent::loop_rs::ToolExecutor::execute(&mut t, &call, &mut |_| {}).unwrap_err()
        );
        assert!(err.contains("历史里没有"), "{err}");
    }

    #[test]
    fn tool_gate_none_is_pass_through() {
        let mut t = crate::server::agent::tools::BuiltinTools::new(std::env::temp_dir());
        assert!(
            t.definitions_for().len() == crate::server::agent::tools::BuiltinTools::definitions().len()
        );
        let call = crate::server::ai::types::ToolCall::new(
            "c1",
            "bash",
            "{\"command\":\"echo hi\",\"intent\":\"x\"}",
        );
        let _ = crate::server::agent::loop_rs::ToolExecutor::execute(&mut t, &call, &mut |_| {}); // runs or fails naturally — not gate-rejected
    }
}
