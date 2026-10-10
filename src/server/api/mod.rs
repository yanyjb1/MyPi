//! api — the client-facing surface and the server's own reuse face: three doors (`call`, `post`,
//! `subscribe`), one folder each. A front end holds no handle to anything else under `server/`.

pub mod call;
mod driver;
pub mod post;
mod reply;
pub mod subscribe;
pub mod types;

pub use reply::stream::StreamItem;
pub use subscribe::Opened;
pub use types::{Call, CandidatePools, ModelId, Post, PostDone, Reply, Subscribe, Viewer};
