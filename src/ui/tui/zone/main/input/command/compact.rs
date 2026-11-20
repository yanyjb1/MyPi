//! `/compact` — fold the history into a checkpoint.

use crate::ui::command::{Arg, CommandMeta};
use crate::ui::lang::Text;
use crate::ui::tui::zone::Frame;
use serde_json::json;

use super::super::Notice;
use super::{Call, Done, Write};

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
    let Some(session) = call.session else {
        return Done::Notice(Notice::NothingToCompact);
    };
    let (head, rest) = match call.args.split_once(char::is_whitespace) {
        Some((head, rest)) => (head, rest.trim()),
        None => (call.args, ""),
    };
    let Some(keep) = keep(head) else {
        let focus = match call.args.is_empty() {
            true => None,
            false => Some(call.args.to_string()),
        };
        return send(session, None, focus);
    };
    let focus = match rest.is_empty() {
        true => None,
        false => Some(rest.to_string()),
    };
    send(session, Some(keep), focus)
}

/// What a compaction keeps: characters of the tail or a block count.
enum Keep {
    Chars(u64),
    Blocks(u64),
}

fn send(session: i64, keep: Option<Keep>, focus: Option<String>) -> Done {
    let mut content = json!({"session": session, "focus": focus});
    if let Some(keep) = &keep {
        content["retention"] = match keep {
            Keep::Chars(n) => json!({"chars": n}),
            Keep::Blocks(n) => json!({"blocks": n}),
        };
    }
    let note = Notice::Compacted {
        chars: match &keep {
            Some(Keep::Chars(n)) => Some(*n),
            _ => None,
        },
        blocks: match &keep {
            Some(Keep::Blocks(n)) => Some(*n),
            _ => None,
        },
        focus: focus.clone(),
    };
    Done::Write(Write::new(
        Frame::new("subscribe", "compact", content),
        note,
    ))
}

/// A character count (`20k`) or a block count (`18`).
fn keep(token: &str) -> Option<Keep> {
    if let Some(count) = token.strip_suffix(['k', 'K']) {
        let count: u64 = count.parse().ok()?;
        return Some(Keep::Chars(count.saturating_mul(1000)));
    }
    let count: u64 = token.parse().ok()?;
    Some(Keep::Blocks(count))
}
