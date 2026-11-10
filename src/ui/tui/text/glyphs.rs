//! The glyph table: plain UTF-8, no Nerd Font needed.

pub(crate) const OK: &str = "[ok]";
pub(crate) const ERR: &str = "[!!]";
pub(crate) const PENDING: &str = "[*]";

/// Four phases, for anything that turns while the session is busy.
pub(crate) const SPINNER: [char; 4] = ['|', '/', '-', '\\'];

/// The starburst the ESC row turns, one frame every 80 ms.
pub(crate) const STAR: [char; 8] = ['✻', '✼', '❉', '❊', '✺', '✹', '✸', '✶'];
pub(crate) const ESC: &str = "ESC";

pub(crate) const SEP: &str = "·";

/// One column of indentation in a code body or a diff line.
pub(crate) const INDENT: &str = "·";

pub(crate) const TODO_PENDING: &str = "[ ]";
pub(crate) const TODO_DONE: &str = "[x]";
