//! What the input zone says after a command: one sentence, keyed for i18n.

use crate::ui::tui::lang::Lang;
use serde_json::{Value, json};

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

impl Notice {
    /// The sentence this language reads.
    pub fn text(&self, lang: Lang) -> String {
        if let Self::Compacted {
            chars,
            blocks,
            focus,
        } = self
        {
            return compacted(lang, chars, blocks, focus);
        }
        let (key, args) = self.keyed();
        crate::i18n::render(lang.code(), key, &args)
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

    /// The i18n key and its holes; `Compacted` is composed in `compacted` instead.
    fn keyed(&self) -> (&'static str, Value) {
        match self {
            Self::Cwd(path) => ("notice_cwd", json!({ "path": path })),
            Self::NoDir(path) => ("notice_no_dir", json!({ "path": path })),
            Self::Named(Some(name)) => ("notice_named", json!({ "name": name })),
            Self::Named(None) => ("notice_named_cleared", json!({})),
            Self::Switched(address) => ("notice_switched", json!({ "address": address })),
            Self::Default(address) => ("notice_default_model", json!({ "address": address })),
            Self::Effort(effort) => ("notice_effort", json!({ "effort": effort })),
            Self::NoEffort => ("effort_unsupported", json!({})),
            Self::NotDeclared(effort) => ("notice_not_declared", json!({ "effort": effort })),
            Self::Refused(text) => ("notice_refused", json!({ "text": text })),
            Self::CwdFallback => ("notice_cwd_fallback", json!({})),
            Self::Compacted { .. } => ("notice_compact_all", json!({})),
            Self::Profile(Some(name)) => ("notice_profile", json!({ "name": name })),
            Self::Profile(None) => ("notice_profile_cleared", json!({})),
            Self::ProfileAfterStart => ("notice_profile_after_start", json!({})),
            Self::NoSession => ("notice_no_session", json!({})),
            Self::NothingToCompact => ("notice_nothing_to_compact", json!({})),
            Self::MemoUsage => ("notice_memo_usage", json!({})),
        }
    }
}

/// The compaction line: a base by retention, then the focus suffix when there is one.
fn compacted(
    lang: Lang,
    chars: &Option<u64>,
    blocks: &Option<u64>,
    focus: &Option<String>,
) -> String {
    let base = match (chars, blocks) {
        (Some(n), _) => {
            crate::i18n::render(lang.code(), "notice_compact_chars", &json!({ "n": n }))
        }
        (_, Some(n)) => {
            crate::i18n::render(lang.code(), "notice_compact_blocks", &json!({ "n": n }))
        }
        _ => crate::i18n::render(lang.code(), "notice_compact_all", &json!({})),
    };
    match focus {
        Some(focus) => {
            let suffix = crate::i18n::render(
                lang.code(),
                "notice_compact_focus",
                &json!({ "focus": focus }),
            );
            format!("{base}{suffix}")
        }
        None => base,
    }
}
