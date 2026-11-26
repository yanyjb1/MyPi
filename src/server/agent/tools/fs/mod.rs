// The fs family: files read and written by line number, and the language rules they lean on.
// Line numbers and byte offsets stay far below integer limits; these casts are intentional.
#![allow(
    clippy::cast_possible_wrap,
    clippy::cast_sign_loss,
    clippy::cast_possible_truncation
)]

pub(crate) mod change;
pub(crate) mod edit;
pub(crate) mod eol;
pub(crate) mod fault;
pub(crate) mod gate;
pub(crate) mod grep;
pub(crate) mod lang;
pub(crate) mod mask;
pub(crate) mod read;
pub(crate) mod skeleton;
pub(crate) mod sniff;
pub(crate) mod splice;
pub(crate) mod write;
