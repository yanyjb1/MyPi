//! `/cdp` — the directory later turns run in.

use crate::ui::command::{Arg, CommandMeta};
use crate::ui::tui::zone::Request;

use super::{Call, Done};

pub const META: CommandMeta = CommandMeta {
    name: "/cdp",
    aliases: &["/cd"],
    args: &[Arg::Path],
    detail: "切换工作目录（持久化，后续相对路径以它为准）",
};

pub fn run(call: &Call<'_>) -> Done {
    if call.args.is_empty() {
        return Done::Hold;
    }
    Done::Intent(Request::SetCwd(call.args.to_string()))
}
