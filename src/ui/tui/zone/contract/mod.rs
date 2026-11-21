// The zone contract: the vocabulary the app and the three major zones share.

mod claim;
mod kind;
mod local;
mod mail;
mod memo;
mod outcome;
mod shared;
mod shout;
mod size;
mod zone;

pub use claim::{Claim, matches};
pub use kind::Kind;
pub use local::Local;
pub use mail::Mail;
pub use memo::Memo;
pub use outcome::{Outcome, ZoneId};
pub use shared::Shared;
pub use shout::{Shout, name};
pub use size::TermSize;
pub use zone::Zone;

// The wire's frame and the session's window, named here so that no zone reaches past this file.
pub use crate::ipc::contract::Frame;
pub use crate::ui::session::Structure;
