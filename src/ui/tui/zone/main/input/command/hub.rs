// `/hub` — this conversation's subagents.

use super::meta::CommandMeta;
use crate::ui::tui::zone::main::input::ExitRequest;

use super::{Call, Done};

pub const META: CommandMeta = CommandMeta {
    name: "/hub",
    aliases: &[],
    args: &[],
    detail: "cmd_hub",
    run,
};

pub fn run(_call: &Call<'_>) -> Done {
    Done::Exit(ExitRequest::OpenHub)
}
