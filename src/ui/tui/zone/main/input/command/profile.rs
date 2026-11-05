//! `/profile` — the prompt a session will run with, chosen before its first message.

use crate::ui::command::{Arg, CommandMeta};

use super::{Call, Done};

pub const META: CommandMeta = CommandMeta {
    name: "/profile",
    aliases: &[],
    args: &[Arg::Profile],
    detail: "切换系统提示词 profile",
};

pub fn run(call: &Call<'_>) -> Done {
    if call.attached {
        return Done::Hint("profile 只能在会话开始时换");
    }
    let name = match call.args {
        "" => None,
        text => Some(text.to_string()),
    };
    Done::Profile(name)
}
