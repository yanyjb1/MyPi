//! `/profile` — the prompt a session will run with, chosen before its first message.

use crate::ui::command::{Arg, CommandMeta};
use crate::ui::lang::Text;

use super::super::Notice;
use super::{Call, Done};

pub const META: CommandMeta = CommandMeta {
    name: "/profile",
    aliases: &[],
    args: &[Arg::Profile],
    detail: Text {
        en: "choose the system-prompt profile a session opens with",
        zh: "选择会话开场用的系统提示词 profile",
    },
};

pub fn run(call: &Call<'_>) -> Done {
    if call.session.is_some() {
        return Done::Notice(Notice::ProfileAfterStart);
    }
    let name = match call.args {
        "" => None,
        text => Some(text.to_string()),
    };
    Done::Profile(name)
}
