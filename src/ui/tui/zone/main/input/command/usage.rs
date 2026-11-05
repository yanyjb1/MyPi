//! `/usage` — the session's bill, as a panel the desk draws.

use crate::ui::command::{Arg, CommandMeta};
use crate::ui::tui::zone::main::desk::Hire;

use super::{Call, Done};

pub const META: CommandMeta = CommandMeta {
    name: "/usage",
    aliases: &[],
    args: &[Arg::Text],
    detail: "看本会话的用量与账单（分模型；lang:zh 用万/亿）",
};

pub fn run(_call: &Call<'_>) -> Done {
    Done::Hire(Hire::Usage)
}
