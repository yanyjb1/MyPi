//! The slash commands this front end knows: one file each, and the table that holds them.

mod call;
mod cd;
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
