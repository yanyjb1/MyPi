//! `/delete` — one stored session goes.

use crate::ui::command::{Arg, CommandMeta};
use crate::ui::tui::zone::Request;

use super::{Call, Done};

pub const META: CommandMeta = CommandMeta {
    name: "/delete",
    aliases: &["/rm"],
    args: &[Arg::Number],
    detail: "删掉一个会话（参数是会话号，一次一个；连带只被它引用的回合、块与巨物）",
};

pub fn run(call: &Call<'_>) -> Done {
    match call.args.parse::<i64>() {
        Ok(session) => Done::Intent(Request::Delete(session)),
        Err(_) => Done::Hold,
    }
}
