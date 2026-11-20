//! `/switch` — this session's model.

use super::meta::{Arg, CommandMeta};
use crate::ui::tui::zone::Frame;
use serde_json::json;

use super::super::super::desk::Effect;
use super::super::Notice;
use super::{Call, Done, Write, pair};

pub const META: CommandMeta = CommandMeta {
    name: "/switch",
    aliases: &[],
    args: &[Arg::Model],
    detail: "cmd_switch",
    run,
};

pub fn run(call: &Call<'_>) -> Done {
    let Some((provider, model)) = pair(call.args) else {
        return Done::Hold;
    };
    let address = format!("{provider}:{model}");
    let note = Notice::Switched(address.clone());
    let Some(session) = call.session else {
        call.draft.set_model(address);
        return Done::Notice(note);
    };
    let letter = Frame::new(
        "post",
        "meta",
        json!({
            "name": "switch",
            "content": {
                "session": session,
                "model": {"provider": provider, "model": model},
            },
        }),
    );
    Done::Write(Write {
        letter,
        effect: Some(Effect::Switch(address)),
        notice: note,
    })
}
