//! `/cdp` — the directory later turns run in.

use std::path::{Path, PathBuf};

use super::meta::{Arg, CommandMeta};
use crate::ui::tui::zone::Frame;
use serde_json::json;

use super::super::Notice;
use super::{Call, Done, Write};

pub const META: CommandMeta = CommandMeta {
    name: "/cdp",
    aliases: &[],
    args: &[Arg::Path],
    detail: "cmd_cdp",
    run,
};

pub fn run(call: &Call<'_>) -> Done {
    if call.args.is_empty() {
        return Done::Hold;
    }
    let Some(target) = resolve(&call.draft.cwd(), call.home, call.args) else {
        return Done::Notice(Notice::NoDir(call.args.to_string()));
    };
    let note = Notice::Cwd(crate::ui::tui::text::short(&target.to_string_lossy()));
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

fn resolve(cwd: &Path, home: &str, raw: &str) -> Option<PathBuf> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let given = match raw.strip_prefix('~') {
        Some(rest) if rest.is_empty() || rest.starts_with(['/', '\\']) => {
            let home = Path::new(home);
            let rest = rest.trim_start_matches(['/', '\\']);
            match rest.is_empty() {
                true => home.to_path_buf(),
                false => home.join(rest),
            }
        }
        // Another user's home is not this process's to expand.
        Some(_) => return None,
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
