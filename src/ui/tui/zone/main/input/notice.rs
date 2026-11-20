//! What the input zone says after a command: one sentence, both languages.

use crate::ui::lang::Lang;

/// One line the input zone puts on the bus: what a command asked for, or why it refused.
#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    /// `/cdp`: the directory asked for.
    Cwd(String),

    /// `/cdp`: no such directory, so the move did not land.
    NoDir(String),

    /// `/name`: the name asked for; `None` clears it.
    Named(Option<String>),

    /// `/switch`: the model this session moves to, as `provider:model`.
    Switched(String),

    /// `/model`: the model a new session opens with, as `provider:model`.
    Default(String),

    /// `/effort`: the effort this session now runs with.
    Effort(String),

    /// `/effort` with a rung this model does not declare.
    NotDeclared(String),

    /// `/effort` on a model whose wire names none.
    NoEffort,

    /// A write the server refused for a reason only it knows.
    Refused(String),

    /// `/cdp` in the init that opened the session: the directory could not be a workspace.
    CwdFallback,

    /// `/compact`: what it keeps, and the focus.
    Compacted {
        chars: Option<u64>,
        blocks: Option<u64>,
        focus: Option<String>,
    },

    /// `/profile`: the prompt the row will open with; `None` clears it.
    Profile(Option<String>),

    /// `/profile` after a session started.
    ProfileAfterStart,

    /// `/usage` or `/compact` with no session open.
    NoSession,

    /// `/compact` with no session open.
    NothingToCompact,

    /// `/memo` without a usable operation.
    MemoUsage,
}

const NO_EFFORT_ZH: &str = "此模型不支持思考强度控制";

impl Notice {
    /// The sentence this language reads.
    pub fn text(&self, lang: Lang) -> String {
        match lang {
            Lang::En => self.en(),
            Lang::Zh => self.zh(),
        }
    }

    /// `info`, or `warn` for a refusal.
    pub fn level(&self) -> &'static str {
        match self {
            Self::ProfileAfterStart
            | Self::NoSession
            | Self::NothingToCompact
            | Self::NoDir(_)
            | Self::Refused(_)
            | Self::NotDeclared(_)
            | Self::CwdFallback
            | Self::MemoUsage => "warn",
            _ => "info",
        }
    }

    fn en(&self) -> String {
        match self {
            Self::Cwd(path) => format!("working directory: {path}"),
            Self::NoDir(path) => format!("no such directory: {path}"),
            Self::Named(Some(name)) => format!("session named {name}"),
            Self::Named(None) => "session name cleared".to_string(),
            Self::Switched(address) => format!("model switched to {address}"),
            Self::Default(address) => format!("default model: {address}"),
            Self::Effort(effort) => format!("thinking effort: {effort}"),
            Self::NoEffort => "this model takes no thinking effort".to_string(),
            Self::NotDeclared(effort) => {
                format!("this model takes no `{effort}` effort")
            }
            Self::Refused(text) => text.clone(),
            Self::CwdFallback => {
                "this directory cannot be a workspace; using the temp directory".to_string()
            }
            Self::Compacted {
                chars,
                blocks,
                focus,
            } => {
                let base = match (chars, blocks) {
                    (Some(n), _) => format!("compacting, keeping about {n} characters"),
                    (_, Some(n)) => format!("compacting, keeping the last {n} blocks"),
                    _ => "compacting the whole history".to_string(),
                };
                match focus {
                    Some(focus) => format!("{base}; focus: {focus}"),
                    None => base,
                }
            }
            Self::Profile(Some(name)) => format!("profile: {name}"),
            Self::Profile(None) => "profile cleared".to_string(),
            Self::ProfileAfterStart => "the profile is fixed once a session starts".to_string(),
            Self::NoSession => "no session yet".to_string(),
            Self::NothingToCompact => "no session yet: nothing to compact".to_string(),
            Self::MemoUsage => "/memo list|history|add|mod|del".to_string(),
        }
    }

    fn zh(&self) -> String {
        match self {
            Self::Cwd(path) => format!("工作目录：{path}"),
            Self::NoDir(path) => format!("目录不存在：{path}"),
            Self::Named(Some(name)) => format!("会话命名为 {name}"),
            Self::Named(None) => "会话名已清除".to_string(),
            Self::Switched(address) => format!("已切换到 {address}"),
            Self::Default(address) => format!("默认模型：{address}"),
            Self::Effort(effort) => format!("思考强度：{effort}"),
            Self::NoEffort => NO_EFFORT_ZH.to_string(),
            Self::NotDeclared(effort) => format!("此模型不支持 `{effort}` 强度"),
            Self::Refused(text) => text.clone(),
            Self::CwdFallback => "该目录不能作为工作区，已改用临时目录".to_string(),
            Self::Compacted {
                chars,
                blocks,
                focus,
            } => {
                let base = match (chars, blocks) {
                    (Some(n), _) => format!("正在压缩，保留约 {n} 字符"),
                    (_, Some(n)) => format!("正在压缩，保留最近 {n} 个块"),
                    _ => "正在压缩全部历史".to_string(),
                };
                match focus {
                    Some(focus) => format!("{base}；焦点：{focus}"),
                    None => base,
                }
            }
            Self::Profile(Some(name)) => format!("profile：{name}"),
            Self::Profile(None) => "profile 已清除".to_string(),
            Self::ProfileAfterStart => "会话开始后不能再换 profile".to_string(),
            Self::NoSession => "还没有会话".to_string(),
            Self::NothingToCompact => "还没有会话，无从压缩".to_string(),
            Self::MemoUsage => "/memo list|history|add|mod|del".to_string(),
        }
    }
}
