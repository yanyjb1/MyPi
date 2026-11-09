//! `/compact` — fold the history into a checkpoint.

use crate::ui::command::{Arg, CommandMeta};
use crate::ui::lang::Text;
use crate::ui::tui::zone::{Request, Retention};

use super::super::Notice;
use super::{Call, Done};

pub const META: CommandMeta = CommandMeta {
    name: "/compact",
    aliases: &[],
    args: &[Arg::Number, Arg::Text],
    detail: Text {
        en: "fold the history into a checkpoint: `/compact 20k` keeps ~20k characters of tail, `/compact 18` keeps the newest 18 blocks, then an optional focus",
        zh: "把历史压缩成检查点：`/compact 20k` 保留约 20k 字符的尾巴，`/compact 18` 保留最新 18 个块，后面还能跟一句焦点",
    },
};

pub fn run(call: &Call<'_>) -> Done {
    if call.session.is_none() {
        return Done::Notice(Notice::NothingToCompact);
    }
    let (head, rest) = match call.args.split_once(char::is_whitespace) {
        Some((head, rest)) => (head, rest.trim()),
        None => (call.args, ""),
    };
    let Some(retention) = retention(head) else {
        let focus = match call.args.is_empty() {
            true => None,
            false => Some(call.args.to_string()),
        };
        return Done::Write {
            request: Request::Compact {
                retention: None,
                focus: focus.clone(),
            },
            notice: Notice::Compacted {
                retention: None,
                focus,
            },
        };
    };
    let focus = match rest.is_empty() {
        true => None,
        false => Some(rest.to_string()),
    };
    Done::Write {
        request: Request::Compact {
            retention: Some(retention),
            focus: focus.clone(),
        },
        notice: Notice::Compacted {
            retention: Some(retention),
            focus,
        },
    }
}

/// A character count (`20k`) or a block count (`18`).
fn retention(token: &str) -> Option<Retention> {
    if let Some(count) = token.strip_suffix(['k', 'K']) {
        let count: u64 = count.parse().ok()?;
        return Some(Retention::Chars(count.saturating_mul(1000)));
    }
    let count: u64 = token.parse().ok()?;
    Some(Retention::Blocks(count))
}
