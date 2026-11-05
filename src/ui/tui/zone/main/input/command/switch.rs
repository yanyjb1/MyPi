//! `/switch` — this session's model.

use crate::ui::command::{Arg, CommandMeta};
use crate::ui::tui::zone::Request;

use super::{Call, Done, pair};

pub const META: CommandMeta = CommandMeta {
    name: "/switch",
    aliases: &[],
    args: &[Arg::Model],
    detail: "切换本会话的模型（落会话状态）",
};

pub fn run(call: &Call<'_>) -> Done {
    match pair(call.args) {
        Some(id) => Done::Intent(Request::SwitchModel(id)),
        None => Done::Hold,
    }
}
