//! The glyph table: ASCII only, so no terminal needs a special font.

pub(super) const OK: &str = "[ok]";
pub(super) const ERR: &str = "[!!]";
pub(super) const PENDING: &str = "[*]";

pub(super) const EXPAND_HINT: &str = "[ctrl+o: Expand]";

pub(super) const SEP: &str = "·";

pub(super) const TODO_PENDING: &str = "[ ]";
pub(super) const TODO_ACTIVE: &str = "[>]";
pub(super) const TODO_DONE: &str = "[x]";
pub(super) const TODO_BLOCKED: &str = "[!]";
pub(super) const TODO_DROPPED: &str = "[-]";

pub(super) const TREE_BRANCH: &str = "|--";

pub(super) const TREE_LAST: &str = "'--";

pub(super) const TREE_VERTICAL: &str = "|";

pub(super) const TREE_HOOK: &str = "`-";
