//! The glyph table: ASCII only, so no terminal needs a special font.

pub(crate) const OK: &str = "[ok]";
pub(crate) const ERR: &str = "[!!]";
pub(crate) const PENDING: &str = "[*]";

pub(crate) const EXPAND_HINT: &str = "[ctrl+o: Expand]";

pub(crate) const SEP: &str = "·";

pub(crate) const TODO_PENDING: &str = "[ ]";
pub(crate) const TODO_ACTIVE: &str = "[>]";
pub(crate) const TODO_DONE: &str = "[x]";
pub(crate) const TODO_BLOCKED: &str = "[!]";
pub(crate) const TODO_DROPPED: &str = "[-]";

pub(crate) const TREE_BRANCH: &str = "|--";

pub(crate) const TREE_LAST: &str = "'--";

pub(crate) const TREE_VERTICAL: &str = "|";

pub(crate) const TREE_HOOK: &str = "`-";
