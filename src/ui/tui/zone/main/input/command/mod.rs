//! The slash commands this front end knows: one file each, and the table that holds them.

mod call;
mod cdp;
mod compact;
mod memo;
mod model;
mod name;
mod new;
mod profile;
mod quit;
mod resume;
mod switch;
mod table;
mod usage;
mod wants;

pub use call::{Call, Done};
pub use table::{TABLE, lookup, pair, run, split};
pub use wants::{slot, wants};
