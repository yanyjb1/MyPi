//! Profiles — swappable system prompts + per-profile tool rosters and
//! compaction instructions.
//!
//! Layout (mirrors the config dir, `~/.config/mypi/profiles/`):
//!
//! ```text
//! profiles/
//!   default/
//!     system.md      ← the system prompt (replaces the built-in one)
//!     tools.yaml     ← optional; enabled tool names (default: all)
//!     compact.md     ← optional; the compaction instruction for this profile
//!   novelist/
//!     system.md
//! ```
//!
//! `system.md` decides whether a directory **is** a profile — it is the
//! identity. The other two are optional side-files; a directory holding only
//! `compact.md` is not a profile and its instruction is not claimed.
//!
//! The look-up order for the compaction instruction is the profile's
//! `compact.md`, then `config.yaml → compact.instruction_file`, then the
//! built-in. The profile's file is read at **every** compaction, so editing it
//! takes effect on the next `/compact`.
//!
//! A profile switch is only legal at a **context rebuild point** —
//! session start, or right after a compaction fork — because the system
//! message is the head of the cached prefix; switching mid-session
//! would cold the KV cache and fork the model's mental state. Enforced
//! by the surface, not this module. The compaction instruction has no such
//! restriction: it never enters the cached prefix.
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
pub const BUILTIN_SYSTEM: &str = "You are a concise coding assistant.";

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
    /// override it; the caller falls through to the config's file, then the
    /// built-in. Read fresh at every compaction (edit the file and the next
    /// `/compact` sees it — same contract as `compact.instruction_file`).
    ///
    /// A profile that names the file but cannot be read is an **error**, not a
    /// silent fall-through: a typo'd path would otherwise compact with a
    /// prompt the user never wrote, and nothing would say so.
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
        let v: serde_yaml::Value =
            serde_yaml::from_str(&text).with_context(|| format!("invalid YAML: {}", p.display()))?;
        let Some(list) = v.get("enabled").and_then(|e| e.as_sequence()) else {
            bail!("tools.yaml needs an `enabled:` list ({})", p.display());
        };
        let names = list
            .iter()
            .filter_map(|i| i.as_str().map(str::to_string))
            .collect::<Vec<_>>();
        if names.is_empty() {
            bail!("tools.yaml has an empty enabled list — keep at least one tool, or drop the file");
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

/// The active profile: config's `profile:` key, or `default`.
pub fn active_name(cfg: &Config) -> String {
    cfg.app.profile.clone().unwrap_or_else(|| "default".into())
}

/// A profile resolved to concrete values. Everything the session needs from
/// it, in one shot: the system prompt, the tool roster, and the path to a
/// compaction instruction (`None` = this profile does not override one).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedProfile {
    pub system: String,
    /// Enabled tool names; `None` = no restriction (all tools).
    pub tools: Option<Vec<String>>,
    /// `profiles/<name>/compact.md`, read at each compaction.
    pub compact_instruction: Option<PathBuf>,
}

/// Resolve the active profile to concrete values.
///
/// Resolution order: matching discovered profile → the `default`
/// profile → the built-in prompt (no dir, no restriction). Missing
/// profile *names* never fail silently here — the caller surfaces the
/// "not found" before reaching this.
pub fn resolve(cfg: &Config, name: &str) -> anyhow::Result<ResolvedProfile> {
    // 内置身份优先：`oh-my-pi` 是随二进制发的提示词，不靠用户的文件系统。
    // 用户若在 profiles 目录里建了同名目录，那就是他显式的**覆盖**——所以
    // 这一支只在目录里没有同名 profile 时生效。
    let base = profiles_dir(cfg)?;
    if let Some(builtin) = builtin_system(name)
        && !discover(&base).iter().any(|p| p.name == name)
    {
        return Ok(ResolvedProfile {
            system: builtin.into(),
            tools: None,
            compact_instruction: None,
        });
    }
    let found = discover(&base);
    if !found.is_empty() || base.is_dir() {
        if let Some(p) = found.iter().find(|p| p.name == name) {
            return Ok(ResolvedProfile {
                system: p.load_system()?,
                tools: p.load_tools()?,
                compact_instruction: p.compact_path.clone(),
            });
        }
        if let Some(builtin) = builtin_system(name) {
            return Ok(ResolvedProfile {
                system: builtin.into(),
                tools: None,
                compact_instruction: None,
            });
        }
        let names = found.iter().map(|p| p.name.as_str()).collect::<Vec<_>>();
        bail!("unknown profile: {name}. Available: {}", names.join(", "));
    }
    // No profiles dir at all: only the built-in identity exists.
    Ok(ResolvedProfile {
        system: BUILTIN_SYSTEM.into(),
        tools: None,
        compact_instruction: None,
    })
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

    /// `compact.md` 是 profile 的第三个标准文件：有就带上，没有就是 `None`
    /// （会话随后退回 config 的 `compact.instruction_file`）。
    #[test]
    fn compact_md_is_discovered_and_loaded() {
        let base = scratch("compact");
        write(&base.join("novelist/system.md"), "小说模式");
        write(&base.join("plain/system.md"), "普通");
        write(&base.join("novelist/compact.md"), "只保留剧情线，丢掉每章的草稿");
        let found = discover(&base);
        let nov = found.iter().find(|p| p.name == "novelist").unwrap();
        assert_eq!(
            nov.load_compact().unwrap().unwrap(),
            "只保留剧情线，丢掉每章的草稿"
        );
        assert!(nov.compact_path.is_some(), "路径本身也要带上：每次压缩重读");
        let plain = found.iter().find(|p| p.name == "plain").unwrap();
        assert!(plain.load_compact().unwrap().is_none());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// 只有 `compact.md` 没有 `system.md` 的目录**不是** profile（`system.md`
    /// 才是身份）。它的压缩指令因此不会被认领——这是有意的：没有系统提示词
    /// 的 profile 没有身份可言。
    #[test]
    fn compact_md_alone_does_not_make_a_profile() {
        let base = scratch("compact-only");
        write(&base.join("orphan/compact.md"), "指令");
        assert!(discover(&base).is_empty());
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
        let default = builtin_system("default").expect("default 是内置的");
        // 内置提示词也是发给模型的，所以和工具描述一样必须是英文：
        // 中文在这里是实打实的 token 税。
        assert!(
            !default.contains('\u{4e00}'),
            "内置系统提示词不该有中文：{default}"
        );
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
        assert!(err.contains("no record of artifact"), "{err}");
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
