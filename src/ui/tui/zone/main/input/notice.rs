//! What the input zone says after a command: one sentence, both languages.

use crate::ui::lang::Lang;
use crate::ui::tui::zone::{ModelId, Retention};

/// One line the input zone puts on the bus: what a command asked for, or why it refused.
#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    /// `/cdp`: the directory asked for.
    Cwd(String),
    /// `/cdp`: no such directory, so the move did not land.
    NoDir(String),
    /// `/name`: the name asked for; `None` clears it.
    Named(Option<String>),
    /// `/switch`: the model this session moves to.
    Switched(ModelId),
    /// `/model`: the model a new session opens with.
    Default(ModelId),
    /// `/compact`: what it keeps, and the focus.
    Compacted {
        retention: Option<Retention>,
        focus: Option<String>,
    },
    /// `/profile` after a session started.
    ProfileAfterStart,
    /// `/usage` or `/compact` with no session open.
    NoSession,
    /// `/compact` with no session open.
    NothingToCompact,
    /// `/memo` without a usable operation.
    MemoUsage,
}

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
            Self::Switched(id) => format!("model switched to {}:{}", id.provider, id.model),
            Self::Default(id) => format!("default model: {}:{}", id.provider, id.model),
            Self::Compacted { retention, focus } => {
                let base = match retention {
                    None => "compacting the whole history".to_string(),
                    Some(Retention::Chars(n)) => {
                        format!("compacting, keeping about {n} characters")
                    }
                    Some(Retention::Blocks(n)) => {
                        format!("compacting, keeping the last {n} blocks")
                    }
                };
                match focus {
                    Some(focus) => format!("{base}; focus: {focus}"),
                    None => base,
                }
            }
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
            Self::Switched(id) => format!("已切换到 {}:{}", id.provider, id.model),
            Self::Default(id) => format!("默认模型：{}:{}", id.provider, id.model),
            Self::Compacted { retention, focus } => {
                let base = match retention {
                    None => "正在压缩全部历史".to_string(),
                    Some(Retention::Chars(n)) => format!("正在压缩，保留约 {n} 字符"),
                    Some(Retention::Blocks(n)) => format!("正在压缩，保留最近 {n} 个块"),
                };
                match focus {
                    Some(focus) => format!("{base}；焦点：{focus}"),
                    None => base,
                }
            }
            Self::ProfileAfterStart => "会话开始后不能再换 profile".to_string(),
            Self::NoSession => "还没有会话".to_string(),
            Self::NothingToCompact => "还没有会话，无从压缩".to_string(),
            Self::MemoUsage => "/memo list|history|add|mod|del".to_string(),
        }
    }
}
