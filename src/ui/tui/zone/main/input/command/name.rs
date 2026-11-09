//! `/name` — the conversation's display name.

use crate::ui::command::{Arg, CommandMeta};
use crate::ui::lang::Text;
use crate::ui::tui::zone::Request;

use super::super::Notice;
use super::{Call, Done};

pub const META: CommandMeta = CommandMeta {
    name: "/name",
    aliases: &[],
    args: &[Arg::Text],
    detail: Text {
        en: "name the session (a branch inherits it; siblings do not see it)",
        zh: "给会话起名（分支继承，兄弟分支看不到）",
    },
};

pub fn run(call: &Call<'_>) -> Done {
    let name = match call.args {
        "" => None,
        text => Some(text.to_string()),
    };
    Done::Write {
        request: Request::Rename(name.clone()),
        notice: Notice::Named(name),
    }
}
