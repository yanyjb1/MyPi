//! `/usage` — the session's bill, as a panel the desk draws.

use super::meta::CommandMeta;
use crate::ui::tui::zone::main::desk::Hire;

use super::super::Notice;
use super::{Call, Done};

pub const META: CommandMeta = CommandMeta {
    name: "/usage",
    aliases: &[],
    args: &[],
    detail: "cmd_usage",
    run,
};

pub fn run(call: &Call<'_>) -> Done {
    if call.session.is_none() {
        return Done::Notice(Notice::NoSession);
    }
    Done::Hire(Hire::Usage)
}
