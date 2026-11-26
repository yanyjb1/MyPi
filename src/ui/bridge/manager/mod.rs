// The sessions the bridge holds: a table of conversations, and one connection behind each.

mod held;
mod hub;

pub use held::Held;
pub use hub::Hub;
