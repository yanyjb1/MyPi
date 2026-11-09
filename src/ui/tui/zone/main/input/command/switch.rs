//! `/switch` — this session's model.

use crate::ui::command::{Arg, CommandMeta};
use crate::ui::lang::Text;
use crate::ui::tui::zone::Request;

use super::super::Notice;
use super::{Call, Done, pair};

pub const META: CommandMeta = CommandMeta {
    name: "/switch",
    aliases: &[],
    args: &[Arg::Model],
    detail: Text {
        en: "switch this session's model (stored on the session)",
        zh: "切换本会话的模型（落会话状态）",
    },
};

pub fn run(call: &Call<'_>) -> Done {
    match pair(call.args) {
        Some(id) => Done::Write {
            request: Request::SwitchModel(id.clone()),
            notice: Notice::Switched(id),
        },
        None => Done::Hold,
    }
}
