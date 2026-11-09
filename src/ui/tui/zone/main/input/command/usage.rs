//! `/usage` — the session's bill, as a panel the desk draws.

use crate::ui::command::CommandMeta;
use crate::ui::lang::Text;
use crate::ui::tui::zone::main::desk::Hire;

use super::super::Notice;
use super::{Call, Done};

pub const META: CommandMeta = CommandMeta {
    name: "/usage",
    aliases: &[],
    args: &[],
    detail: Text {
        en: "this session's usage and bill, per model",
        zh: "看本会话的用量与账单（分模型）",
    },
};

pub fn run(call: &Call<'_>) -> Done {
    if call.session.is_none() {
        return Done::Notice(Notice::NoSession);
    }
    Done::Hire(Hire::Usage)
}
