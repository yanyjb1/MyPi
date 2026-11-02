//! The zone contract: the only vocabulary the app and the zones share.

mod claim;
mod frame;
mod kind;
mod memo;
mod outcome;
mod request;
mod shout;
mod size;
mod turns;
mod zone;

pub use claim::{Claim, matches};
pub use frame::Frame;
pub use kind::Kind;
pub use memo::Memo;
pub use outcome::{Outcome, ZoneId};
pub use request::{ModelId, Request};
pub use shout::{Shout, name};
pub use size::TermSize;
pub use turns::{Round, Turns};
pub use zone::{SubZone, Zone};
