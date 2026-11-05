//! `/resume` — the session picker.

use crate::ui::command::{Arg, CommandMeta};
use crate::ui::tui::zone::main::input::ExitRequest;

use super::{Call, Done};

pub const META: CommandMeta = CommandMeta {
    name: "/resume",
    aliases: &[],
    args: &[],
    detail: "恢复本项目的某个会话",
};

pub fn run(_call: &Call<'_>) -> Done {
    Done::Exit(ExitRequest::OpenResume)
}
