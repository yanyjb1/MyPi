//! `/model` — the model a new session opens with; the config file is the record.

use crate::ui::command::{Arg, CommandMeta};
use crate::ui::lang::Text;
use crate::ui::tui::zone::Request;

use super::super::Notice;
use super::{Call, Done, pair};

pub const META: CommandMeta = CommandMeta {
    name: "/model",
    aliases: &[],
    args: &[Arg::Model],
    detail: Text {
        en: "switch model and write it as the default (config file; a new session opens with it)",
        zh: "换模型并写为默认（改配置文件，新会话以它开场）",
    },
};

pub fn run(call: &Call<'_>) -> Done {
    match pair(call.args) {
        Some(id) => Done::Write {
            request: Request::SetDefaultModel(id.clone()),
            notice: Notice::Default(id),
        },
        None => Done::Hold,
    }
}
