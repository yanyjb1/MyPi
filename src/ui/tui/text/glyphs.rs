//! The glyph table: plain UTF-8, no Nerd Font needed.

pub(crate) const OK: &str = "[ok]";
pub(crate) const ERR: &str = "[!!]";
pub(crate) const PENDING: &str = "[*]";
/// The user stopped the turn.
pub(crate) const STOPPED: &str = "[-]";
/// The call's own deadline passed.
pub(crate) const TIMEOUT: &str = "[!]";

pub(crate) const SEP: &str = "·";

/// One column of indentation in a code body or a diff line.
pub(crate) const INDENT: &str = "·";

pub(crate) const TODO_PENDING: &str = "[ ]";
pub(crate) const TODO_DONE: &str = "[x]";
