//! `/profile` — the prompt a session will run with, chosen before its first message.

use super::meta::{Arg, CommandMeta};

use super::super::Notice;
use super::{Call, Done};

pub const META: CommandMeta = CommandMeta {
    name: "/profile",
    aliases: &[],
    args: &[Arg::Profile],
    detail: "cmd_profile",
    run,
};

pub fn run(call: &Call<'_>) -> Done {
    if call.session.is_some() {
        return Done::Notice(Notice::ProfileAfterStart);
    }
    let name = match call.args {
        "" => None,
        text => Some(text.to_string()),
    };
    call.draft.set_profile(name.clone());
    Done::Notice(Notice::Profile(name))
}
