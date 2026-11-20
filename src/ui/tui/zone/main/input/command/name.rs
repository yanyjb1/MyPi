//! `/name` — the conversation's display name.

use super::meta::{Arg, CommandMeta};
use crate::ui::lang::Text;
use crate::ui::tui::zone::Frame;
use serde_json::json;

use super::super::super::desk::Effect;
use super::super::Notice;
use super::{Call, Done, Write};

pub const META: CommandMeta = CommandMeta {
    name: "/name",
    aliases: &[],
    args: &[Arg::Text],
    detail: Text {
        en: "name the session (a branch inherits it; siblings do not see it)",
        zh: "给会话起名（分支继承，兄弟分支看不到）",
    },
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
