//! 斜杠命令表 —— **服务端**的命令清单，前端只消费它的元数据。
//!
//! 命令是**会话操作**（改名、切目录、压缩），而会话真相在服务端；所以表与
//! 分发都在这里，前端只做两件事：把 `/name args` 认出来发过来，以及拿这张表
//! 做补全。omp 的 `packages/coding-agent/src/slash-commands/` 是同一个分法：
//! 元数据 UI 无关、handler 在服务端、TUI 与 ACP 两个前端共用一张表。
//!
//! 三类命令，用 [`Scope`] 分开：
//!
//! - [`Scope::Session`] —— 服务端执行，结果以一条 `Entry::System` 回灌转录；
//! - [`Scope::Local`] —— 前端自己的事（退出、会话选择器这种要画 overlay 的），
//!   服务端只提供元数据让它认识这个名字。
//!
//! 一个名字只能有一个归属：把 `/cd` 放前端做，就和"服务端持有 cwd 真相"打架。

/// 命令参数的形状：补全与校验共用。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArgKind {
    /// 没有参数，命令名就是终点。
    None,
    /// 参数是模型 id（候选来自运行时配置）。
    ModelId,
    /// 参数是 profile 名（候选来自运行时配置里的花名册）。
    ProfileName,
    /// 参数是路径（复用文件补全）。
    Path,
    /// 参数是自由文本（焦点、名字……）。
    Text,
}

/// 这条命令归谁执行。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// 服务端：会话操作，结果回灌转录。
    Session,
    /// 前端：纯本地（退出、选择器）。
    Local,
}

/// 一条斜杠命令的元数据。**UI 无关**：任何前端都能拿它做补全、帮助、参数提示。
pub struct CommandSpec {
    /// 带斜杠的名字，例如 `/name`。
    pub name: &'static str,
    /// 别名（同一个 handler，不同的写法）。
    pub aliases: &'static [&'static str],
    /// 补全与帮助里显示的一句话。
    pub detail: &'static str,
    pub args: ArgKind,
    pub scope: Scope,
}

/// 命令表。顺序 = 补全里的顺序。
pub const COMMANDS: &[CommandSpec] = &[
    CommandSpec {
        name: "/name",
        aliases: &[],
        detail: "给会话起名（分支继承，兄弟分支看不到）",
        args: ArgKind::Text,
        scope: Scope::Session,
    },
    CommandSpec {
        name: "/cdp",
        aliases: &["/cd"],
        detail: "切换工作目录（持久化，后续相对路径以它为准）",
        args: ArgKind::Path,
        scope: Scope::Session,
    },
    CommandSpec {
        name: "/compact",
        aliases: &[],
        detail: "把历史压缩成检查点（可带一句焦点）",
        args: ArgKind::Text,
        scope: Scope::Session,
    },
    CommandSpec {
        name: "/switch",
        aliases: &[],
        detail: "切换本会话的模型（不落盘）",
        args: ArgKind::ModelId,
        scope: Scope::Session,
    },
    CommandSpec {
        name: "/profile",
        aliases: &[],
        detail: "切换系统提示词 profile",
        args: ArgKind::ProfileName,
        scope: Scope::Session,
    },
    CommandSpec {
        name: "/model",
        aliases: &[],
        detail: "换模型并写为默认（改配置文件，下次启动生效）",
        args: ArgKind::ModelId,
        scope: Scope::Session,
    },
    CommandSpec {
        name: "/resume",
        aliases: &[],
        detail: "恢复本项目的某个会话",
        args: ArgKind::None,
        scope: Scope::Local,
    },
    CommandSpec {
        name: "/q",
        aliases: &["/quit", "/exit"],
        detail: "退出",
        args: ArgKind::None,
        scope: Scope::Local,
    },
];

/// Everything a **session-scoped** command needs from the outside world.
///
/// One struct instead of a growing parameter list on `Session`: a command that
/// needs configuration declares it here once, and `Session::run_command` stays a
/// match over operations. `None` means "not wired" — the command says so plainly
/// instead of pretending to have run (see `Session::run_command`).
///
/// Closures rather than a `Config` handle: the session layer has no business
/// knowing how models or profiles are stored, and tests can hand it a two-line
/// stand-in.
#[derive(Clone, Default)]
pub struct CommandEnv {
    /// Resolve a model id (`<provider>:<id>`) into everything `/switch` needs.
    pub resolve_model: Option<ModelResolver>,
    /// Resolve a profile name into (system prompt, tool roster).
    pub resolve_profile: Option<ProfileResolver>,
    /// Persist a model id as the default — `/model`'s second landing point.
    ///
    /// An interface, not a path: the session never learns *where* the default
    /// lives or what file format holds it (config.yaml is due for a rewrite).
    /// Returns a human-readable destination for the notice.
    pub set_default_model: Option<DefaultModelWriter>,
}

/// `Fn(&str) -> Result<ModelSwitch, String>`: the id the user typed, the pieces
/// the session needs to actually switch to it.
pub type ModelResolver =
    std::sync::Arc<dyn Fn(&str) -> Result<ModelSwitch, String> + Send + Sync>;

/// `Fn(model id) -> Result<where it went, why not>`.
pub type DefaultModelWriter =
    std::sync::Arc<dyn Fn(&str) -> Result<String, String> + Send + Sync>;

/// `Fn(&str) -> Result<(system prompt, tool roster), String>`.
pub type ProfileResolver =
    std::sync::Arc<dyn Fn(&str) -> Result<(String, Option<Vec<String>>), String> + Send + Sync>;

/// A model, resolved down to what the client and the status line need.
///
/// Flattened on purpose: the session must not learn about providers, key
/// resolution or the models file — those are the caller's problem, solved once.
#[derive(Debug, Clone)]
pub struct ModelSwitch {
    /// Provider base URL (the client's endpoint changes with the model).
    pub base_url: String,
    pub api_key: String,
    /// The id sent on the wire.
    pub model_id: String,
    /// Statusline display name (models.yml `name`, else the id).
    pub name: String,
    /// Context window for the usage gauge (0 = unknown).
    pub context_window: u64,
    /// Output ceiling for the next requests.
    pub max_tokens: u32,
    /// Price sheet of the new model.
    pub cost: crate::server::ai::config::Cost,
}

/// 按名字或别名查一条命令。
pub fn lookup(name: &str) -> Option<&'static CommandSpec> {
    COMMANDS
        .iter()
        .find(|c| c.name == name || c.aliases.contains(&name))
}

/// 一个 `/` 开头的输入是不是命令（且被认识）。
///
/// 不认识的 `/xxx` 由调用方**明确告知**「未知命令」，不能静默当用户消息发给
/// 模型——那既浪费 token，又让模型对着一句它无法执行的指令瞎猜。
pub fn split(input: &str) -> Option<(&'static CommandSpec, &str)> {
    let input = input.trim();
    let rest = input.strip_prefix('/')?;
    if rest.is_empty() {
        return None;
    }
    let (name, args) = match rest.split_once(char::is_whitespace) {
        Some((n, a)) => (format!("/{n}"), a.trim()),
        None => (format!("/{rest}"), ""),
    };
    // 只认「名字是完整词」：`/nameless` 不是 `/name`。
    lookup(&name).map(|c| (c, args))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_aliases_resolve_to_the_same_spec() {
        assert_eq!(lookup("/name").unwrap().name, "/name");
        assert_eq!(lookup("/cd").unwrap().name, "/cdp");
        assert_eq!(lookup("/quit").unwrap().name, "/q");
        assert!(lookup("/nope").is_none());
    }

    #[test]
    fn splitting_keeps_the_argument_and_rejects_lookalikes() {
        let (spec, args) = split("/name 重构工具层").unwrap();
        assert_eq!(spec.name, "/name");
        assert_eq!(args, "重构工具层");

        // 没有参数
        let (spec, args) = split("/compact").unwrap();
        assert_eq!(spec.name, "/compact");
        assert_eq!(args, "");

        // 名字必须是完整词：`/nameless` 不是 `/name`
        assert!(split("/nameless").is_none());
        // 不是命令
        assert!(split("name").is_none());
        assert!(split("/").is_none());
    }

    #[test]
    fn every_command_declares_its_scope_and_a_detail() {
        // 元数据是前端补全与帮助的唯一来源，缺了就是空行。
        let mut seen = std::collections::BTreeSet::new();
        for c in COMMANDS {
            assert!(c.name.starts_with('/'), "{} 该带斜杠", c.name);
            assert!(seen.insert(c.name), "命令名重复: {}", c.name);
            assert!(!c.detail.trim().is_empty(), "{} 缺说明", c.name);
            for a in c.aliases {
                assert!(a.starts_with('/'), "{a} 该带斜杠");
                assert!(seen.insert(a), "别名重复或与命令名撞车: {a}");
            }
        }
    }

    #[test]
    fn quitting_is_a_local_command() {
        // 退出是前端自己的事：服务端连上就不该替前端决定它要不要退出。
        assert_eq!(lookup("/q").unwrap().scope, Scope::Local);
        assert_eq!(lookup("/resume").unwrap().scope, Scope::Local);
        assert_eq!(lookup("/model").unwrap().scope, Scope::Session);
        assert_eq!(lookup("/name").unwrap().scope, Scope::Session);
        assert_eq!(lookup("/compact").unwrap().scope, Scope::Session);
    }
}
