//! `/name` — the conversation's display name.

use super::meta::{Arg, CommandMeta};
use crate::ui::tui::zone::Frame;
use serde_json::json;

use super::super::super::desk::Effect;
use super::super::Notice;
use super::{Call, Done, Write};

pub const META: CommandMeta = CommandMeta {
    name: "/name",
    aliases: &[],
    args: &[Arg::Text],
    detail: "cmd_name",
    run,
};

pub fn run(call: &Call<'_>) -> Done {
    let name = match call.args {
        "" => None,
        text => Some(text.to_string()),
    };
    let note = Notice::Named(name.clone());
    let Some(session) = call.session else {
        call.draft.set_name(name);
        return Done::Notice(note);
    };
    let letter = Frame::new(
        "post",
        "meta",
        json!({
            "name": "name",
            "content": {"session": session, "name": name},
        }),
    );
    Done::Write(Write {
        letter,
        effect: Some(Effect::Rename(name)),
        notice: note,
    })
}
