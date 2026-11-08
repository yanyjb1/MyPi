//! Post — the write side: one door per name, each gated on the session's writer.

pub(crate) mod cwd;
pub(crate) mod default_model;
pub(crate) mod delete;
pub(crate) mod emit;
pub(crate) mod init;
pub(crate) mod interrupt;
pub(crate) mod mailbox;
pub(crate) mod memo;
pub(crate) mod name;
pub(crate) mod rewind;
pub(crate) mod switch;
