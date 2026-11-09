//! Does this position want completion? One rule, for the hire and for the candidates alike.

/// Pure: it reads the line and nothing else. A word only shaped like a path still counts —
/// whether the directory exists is the listing's business, not the hire's.
pub fn wants(before: &str, word: &str) -> bool {
    use crate::ui::command::Arg;
    if before.starts_with('/') && !before.contains('\n') {
        return match slot(before) {
            None => true,
            Some(arg) => matches!(arg, Some(Arg::Path | Arg::Model | Arg::Profile)),
        };
    }
    crate::ui::command::path_shaped(word)
}

/// The argument kind the cursor sits on; `None` = the cursor is on the command name.
pub fn slot(before: &str) -> Option<Option<crate::ui::command::Arg>> {
    let index = crate::ui::command::word_index(before);
    if index == 0 {
        return None;
    }
    let name = before.split_whitespace().next()?;
    Some(super::lookup(name)?.arg(index - 1))
}
