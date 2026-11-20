//! `/model` — the model a new session opens with; the config file is the record.

use super::meta::{Arg, CommandMeta};
use crate::ui::lang::Text;
use crate::ui::tui::zone::Frame;
use serde_json::json;

use super::super::Notice;
use super::{Call, Done, Write, pair};

pub const META: CommandMeta = CommandMeta {
    name: "/model",
    aliases: &[],
    args: &[Arg::Model],
    detail: Text {
        en: "switch model and write it as the default (config file; a new session opens with it)",
        zh: "换模型并写为默认（改配置文件，新会话以它开场）",
    },
    run,
};

pub fn run(call: &Call<'_>) -> Done {
    let Some((provider, model)) = pair(call.args) else {
        return Done::Hold;
    };
    let address = format!("{provider}:{model}");
    call.draft.set_model(address.clone());
    let letter = Frame::new(
        "post",
        "default_model",
        json!({"model": {"provider": provider, "model": model}}),
    );
    Done::Write(Write::new(letter, Notice::Default(address)))
}
