//! `/new` — a fresh conversation, without attaching another.

use crate::ui::command::CommandMeta;
use crate::ui::tui::zone::main::input::ExitRequest;

use super::{Call, Done};

pub const META: CommandMeta = CommandMeta {
    name: "/new",
    aliases: &[],
    args: &[],
    detail: "开一个新会话",
};

pub fn run(_call: &Call<'_>) -> Done {
    Done::Exit(ExitRequest::New)
}
