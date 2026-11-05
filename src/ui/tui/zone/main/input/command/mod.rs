//! The slash commands this front end knows: one file each, and the table that holds them.

mod call;
mod cdp;
mod compact;
mod delete;
mod model;
mod name;
mod new;
mod profile;
mod quit;
mod resume;
mod switch;
mod table;
mod usage;

pub use call::{Call, Done};
pub use table::{TABLE, lookup, pair, run, split};

/// Does this position want completion? One rule, for the hire and for the candidates alike.
///
/// Pure: it reads the line and nothing else. A word only shaped like a path still counts —
/// whether the directory exists is the listing's business, not the hire's.
pub fn wants(before: &str, word: &str) -> bool {
    if before.starts_with('/') && !before.contains('\n') {
        return match argument(before) {
            None => true,
            Some(arg) => matches!(
                arg,
                Some(
                    crate::ui::command::Arg::Path
                        | crate::ui::command::Arg::Model
                        | crate::ui::command::Arg::Profile
                )
            ),
        };
    }
    crate::ui::command::path_shaped(word)
}

/// The argument position the cursor sits in: `None` = the cursor is on the command name.
fn argument(before: &str) -> Option<Option<crate::ui::command::Arg>> {
    let index = crate::ui::command::word_index(before);
    if index == 0 {
        return None;
    }
    let name = before.split_whitespace().next()?;
    Some(lookup(name)?.arg(index - 1))
}
