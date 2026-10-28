//! The wire contract: every shape the server and a front end both name. This module may not
//! reach for `server`, `ui` or `ipc` transport code; those three meet here, not in each other.

pub mod call;
pub mod data;
pub mod envelope;
pub mod post;
pub mod stream;

pub use call::{
    Call, CandidatePools, ChildRow, ChildState, ConfigValue, InitReply, MemoData, MemoEntry,
    MemoOp, MemoScope, ModelUsage, Reply, ServerAsk, ServerReply, SessionAsk, SessionReply,
    SessionRow, SessionState, ToolAsk, ToolReply, TurnBlock, TurnShape, UsageCost, UsageLang,
    UsageReply,
};
pub use data::{
    Block, BlockKind, ContentKey, NewBlock, SessionMeta, Spill, TodoPhase, TodoStatus, TodoTask,
    display_name,
};
pub use envelope::{Answer, Ask, ErrorCode, PROTOCOL_VERSION, Viewer};
pub use post::{
    MemoWrite, MemoWriteOp, ModelId, Post, PostDone, ServerWrite, SessionWrite, ToolWrite,
};
pub use stream::{
    ContentEvent, Event, Notice, NoticeLevel, Opened, Retention, SessionEvent, StartKind,
    Subscribe, TurnUsage,
};
