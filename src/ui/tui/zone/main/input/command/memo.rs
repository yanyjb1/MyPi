//! `/memo` — the notes panel: list, one name's versions, writes, and a delete.

use crate::ui::command::{Arg, CommandMeta};
use crate::ui::lang::Text;
use crate::ui::tui::zone::main::desk::Hire;
use crate::ui::tui::zone::{MemoAsk, MemoScope};

use super::super::Notice;
use super::{Call, Done};

pub const META: CommandMeta = CommandMeta {
    name: "/memo",
    aliases: &[],
    args: &[Arg::Text],
    detail: Text {
        en: "memos: list, history, add, mod, del; scope defaults to global",
        zh: "备忘录：list、history、add、mod、del；scope 默认 global",
    },
};

pub fn run(call: &Call<'_>) -> Done {
    let rest = call.args.trim();
    if rest.is_empty() {
        return panel(MemoAsk::List);
    }
    let (op, rest) = split(rest);
    match op {
        "list" => panel(MemoAsk::List),
        "history" => {
            let (scope, rest) = take_scope(rest);
            let (name, _) = split(rest);
            match name.is_empty() {
                true => Done::Hold,
                false => panel(MemoAsk::History {
                    scope,
                    name: name.to_string(),
                }),
            }
        }
        "del" => {
            let (scope, rest) = take_scope(rest);
            let (name, _) = split(rest);
            match name.is_empty() {
                true => Done::Hold,
                false => panel(MemoAsk::Del {
                    scope,
                    name: name.to_string(),
                }),
            }
        }
        "add" | "mod" => {
            let (scope, rest) = take_scope(rest);
            let (name, content) = split(rest);
            let content = content.trim();
            if name.is_empty() || content.is_empty() {
                return Done::Hold;
            }
            let (name, content) = (name.to_string(), content.to_string());
            panel(match op {
                "add" => MemoAsk::Add {
                    scope,
                    name,
                    content,
                },
                _ => MemoAsk::Mod {
                    scope,
                    name,
                    content,
                },
            })
        }
        _ => Done::Notice(Notice::MemoUsage),
    }
}

fn panel(ask: MemoAsk) -> Done {
    Done::Hire(Hire::Memo(ask))
}

/// One word and the rest of the line, left trimmed.
fn split(text: &str) -> (&str, &str) {
    match text.split_once(char::is_whitespace) {
        Some((word, rest)) => (word, rest.trim_start()),
        None => (text, ""),
    }
}

/// A leading `global` or `current`; anything else leaves the text and picks global.
fn take_scope(text: &str) -> (MemoScope, &str) {
    let (word, rest) = split(text);
    match word {
        "current" => (MemoScope::Current, rest),
        "global" => (MemoScope::Global, rest),
        _ => (MemoScope::Global, text),
    }
}
