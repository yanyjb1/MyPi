//! `/resume` — the session picker.

use super::meta::CommandMeta;
use crate::ui::tui::zone::main::input::ExitRequest;

use super::{Call, Done};

pub const META: CommandMeta = CommandMeta {
    name: "/resume",
    aliases: &[],
    args: &[],
    detail: "cmd_resume",
    run,
};

pub fn run(_call: &Call<'_>) -> Done {
    Done::Exit(ExitRequest::OpenResume)
}
