//! `/effort` — how hard this session asks the model to think.

use super::meta::{Arg, CommandMeta};
use crate::ui::tui::zone::Frame;
use serde_json::json;

use super::super::super::desk::Effect;
use super::super::Notice;
use super::{Call, Done, Write};

pub const META: CommandMeta = CommandMeta {
    name: "/effort",
    aliases: &[],
    args: &[Arg::Effort],
    detail: "cmd_effort",
    run,
};

pub fn run(call: &Call<'_>) -> Done {
    // A model that declares no rung cannot be asked for one, and a rung it never declared is
    // refused here, before the letter reaches a host that would answer with a 400.
    if call.efforts.is_empty() {
        return Done::Notice(Notice::NoEffort);
    }
    let rung = call.args.trim().to_ascii_lowercase();
    if rung.is_empty() {
        return Done::Hold;
    }
    if rung != "auto" && !call.efforts.iter().any(|declared| declared == &rung) {
        return Done::Notice(Notice::NotDeclared(rung));
    }
    let note = Notice::Effort(rung.clone());
    let Some(session) = call.session else {
        call.draft.set_effort(rung);
        return Done::Notice(note);
    };
    let letter = Frame::new(
        "post",
        "meta",
        json!({
            "name": "effort",
            "content": {"session": session, "effort": rung},
        }),
    );
    Done::Write(Write {
        letter,
        effect: Some(Effect::Effort(rung)),
        notice: note,
    })
}
