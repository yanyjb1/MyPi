//! The wire contract: every shape the server and a front end both name. This module may not
//! reach for `server`, `ui` or `ipc` transport code; those three meet here, not in each other.

pub mod call;
pub mod data;
pub mod error;
pub mod message;
pub mod notice;
pub mod post;
pub mod register;
pub mod subscribe;

pub use call::children::{ChildRow, ChildState};
pub use call::sessions::SessionRow;
pub use data::{
    Block, BlockKind, ContentKey, NewBlock, SessionMeta, Spill, TodoPhase, TodoStatus, TodoTask,
    display_name,
};
pub use error::{ErrorCode, ErrorPayload};
pub use message::{Frame, Kind};
pub use notice::{Notice, NoticeLevel};
pub use register::Viewer;
pub use subscribe::{Retention, StartKind, TurnUsage};
