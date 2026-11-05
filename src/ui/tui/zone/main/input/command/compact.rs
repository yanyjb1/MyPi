//! `/compact` — fold the history into a checkpoint.

use crate::ui::command::{Arg, CommandMeta};
use crate::ui::tui::zone::{Request, Retention};

use super::{Call, Done};

pub const META: CommandMeta = CommandMeta {
    name: "/compact",
    aliases: &[],
    args: &[Arg::Number, Arg::Text],
    detail: "把历史压缩成检查点：`/compact 20k` 保留约 20k token 的尾巴，`/compact 18` 保留最新 18 个块，后面还能跟一句焦点",
};

pub fn run(call: &Call<'_>) -> Done {
    let (head, rest) = match call.args.split_once(char::is_whitespace) {
        Some((head, rest)) => (head, rest.trim()),
        None => (call.args, ""),
    };
    let Some(retention) = retention(head) else {
        let focus = match call.args.is_empty() {
            true => None,
            false => Some(call.args.to_string()),
        };
        return Done::Intent(Request::Compact {
            retention: None,
            focus,
        });
    };
    let focus = match rest.is_empty() {
        true => None,
        false => Some(rest.to_string()),
    };
    Done::Intent(Request::Compact {
        retention: Some(retention),
        focus,
    })
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
