//! The glyph table: ASCII only, so no terminal needs a special font.

pub(crate) const OK: &str = "[ok]";
pub(crate) const ERR: &str = "[!!]";
pub(crate) const PENDING: &str = "[*]";

pub(crate) const SEP: &str = "·";

pub(crate) const TODO_PENDING: &str = "[ ]";
pub(crate) const TODO_ACTIVE: &str = "[>]";
pub(crate) const TODO_DONE: &str = "[x]";
pub(crate) const TODO_BLOCKED: &str = "[!]";
pub(crate) const TODO_DROPPED: &str = "[-]";
