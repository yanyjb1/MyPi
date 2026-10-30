//! One outgoing call: the wire shapes, the decode, and the blocks it leaves behind.

pub mod artifact;
pub mod ask;
pub mod assembly;
pub mod compact;
pub mod contract;
pub mod live;
pub mod message;
pub mod protocol;
pub mod retry;
pub mod transcript;
pub mod transport;

pub use ask::{Options, Request, Streaming};
pub use contract::{
    Artifact, Ask, Block, BlockSink, Borrow, CompactInput, CompactReply, History, Kind, LapInput,
    LapReply, Piece, Reference, ResultsInput, Retention, Spill, Stop, ToolCall, ToolResult,
    Transport, TurnView,
};
pub use live::{Element, Event, Notice, NoticeLevel, Stream, ToolEvent};
pub use message::{AssistantMessage, Message, Usage, WireMessage};
pub use protocol::ProtocolId;
pub use retry::RetryPolicy;
pub use transport::Provider;
