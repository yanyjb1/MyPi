//! `/name` — the conversation's display name.

use crate::ui::command::{Arg, CommandMeta};
use crate::ui::tui::zone::Request;

use super::{Call, Done};

pub const META: CommandMeta = CommandMeta {
    name: "/name",
    aliases: &[],
    args: &[Arg::Text],
    detail: "给会话起名（分支继承，兄弟分支看不到）",
};

pub fn run(call: &Call<'_>) -> Done {
    let name = match call.args {
        "" => None,
        text => Some(text.to_string()),
    };
    Done::Intent(Request::Rename(name))
}
