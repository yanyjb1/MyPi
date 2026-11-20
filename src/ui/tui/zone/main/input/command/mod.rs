//! The slash commands this front end knows: one file each, and the table that holds them.

mod call;
mod cdp;
mod compact;
mod effort;
mod memo;
mod meta;
mod model;
mod name;
mod new;
mod prepare;
mod profile;
mod quit;
mod resume;
mod switch;
mod table;
mod usage;
mod wants;

pub use call::{Call, Done, Write};
pub use meta::{Arg, CommandMeta, path_shaped, word, word_index};
pub(crate) use prepare::payload;
pub use table::{TABLE, lookup, pair, split};
pub use wants::{slot, wants};
