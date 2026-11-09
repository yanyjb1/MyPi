//! `/cdp` — the directory later turns run in.

use crate::ui::command::{Arg, CommandMeta};
use crate::ui::lang::Text;
use crate::ui::tui::zone::Request;

use super::super::Notice;
use super::{Call, Done};

pub const META: CommandMeta = CommandMeta {
    name: "/cdp",
    aliases: &[],
    args: &[Arg::Path],
    detail: Text {
        en: "switch the working directory; later relative paths resolve from it",
        zh: "切换工作目录（持久化，后续相对路径以它为准）",
    },
};

pub fn run(call: &Call<'_>) -> Done {
    if call.args.is_empty() {
        return Done::Hold;
    }
    Done::Write {
        request: Request::SetCwd(call.args.to_string()),
        notice: Notice::Cwd(call.args.to_string()),
    }
}
