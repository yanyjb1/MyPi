//! `/q` — leave.

use crate::ui::command::{Arg, CommandMeta};
use crate::ui::tui::zone::main::input::ExitRequest;

use super::{Call, Done};

pub const META: CommandMeta = CommandMeta {
    name: "/q",
    aliases: &["/quit", "/exit"],
    args: &[],
    detail: "退出",
};

pub fn run(_call: &Call<'_>) -> Done {
    Done::Exit(ExitRequest::Quit)
}
