//! `/model` — the model a new session opens with; the config file is the record.

use crate::ui::command::{Arg, CommandMeta};
use crate::ui::tui::zone::Request;

use super::{Call, Done, pair};

pub const META: CommandMeta = CommandMeta {
    name: "/model",
    aliases: &[],
    args: &[Arg::Model],
    detail: "换模型并写为默认（改配置文件，下次启动生效）",
};

pub fn run(call: &Call<'_>) -> Done {
    match pair(call.args) {
        Some(id) => Done::Intent(Request::SetDefaultModel(id)),
        None => Done::Hold,
    }
}
