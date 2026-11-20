//! `/cdp` — the directory later turns run in.

use std::path::{Path, PathBuf};

use crate::ui::command::{Arg, CommandMeta};
use crate::ui::lang::Text;
use crate::ui::tui::zone::Frame;
use serde_json::json;

use super::super::Notice;
use super::{Call, Done, Write};

pub const META: CommandMeta = CommandMeta {
    name: "/cdp",
    aliases: &[],
    args: &[Arg::Path],
    detail: Text {
        en: "switch the working directory; later relative paths resolve from it",
        zh: "切换工作目录（持久化，后续相对路径以它为准）",
    },
};

pub fn run(call: &Call<'_>) -> Done {
    if call.args.is_empty() {
        return Done::Hold;
    }
    let Some(target) = resolve(&call.draft.cwd(), call.args) else {
        return Done::Notice(Notice::NoDir(call.args.to_string()));
    };
    let note = Notice::Cwd(target.to_string_lossy().to_string());
    let Some(session) = call.session else {
        call.draft.land(target);
        return Done::Notice(note);
    };
    let letter = Frame::new(
        "post",
        "meta",
        json!({
            "name": "cwd",
            "content": {"session": session, "path": target.to_string_lossy()},
        }),
    );
    Done::Write(Write::new(letter, note))
}

/// The directory a `/cdp` lands on: `~` expanded, a relative path read from where the draft
/// stands, and the answer MUST be a directory that exists. `None` = it is not there.
fn resolve(cwd: &Path, raw: &str) -> Option<PathBuf> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let given = match raw.strip_prefix('~') {
        Some(rest) => std::env::var_os("HOME")
            .map(PathBuf::from)?
            .join(rest.trim_start_matches('/')),
        None => PathBuf::from(raw),
    };
    let given = if given.is_absolute() {
        given
    } else {
        cwd.join(given)
    };
    let real = given.canonicalize().ok()?;
    real.is_dir().then_some(real)
}
