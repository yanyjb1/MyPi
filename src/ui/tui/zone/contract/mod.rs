//! The zone contract: the only vocabulary the app and the zones share.

mod claim;
mod kind;
mod memo;
mod outcome;
mod request;
mod size;
mod zone;

pub use claim::Claim;
pub use kind::Kind;
pub use memo::Memo;
pub use outcome::{Outcome, ZoneId};
pub use request::{ModelId, Request};
pub use size::TermSize;
pub use zone::{SubZone, Zone};

pub use crate::ui::session::{Packet, Structure};
