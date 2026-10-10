//! ipc — the wire between the two mypi processes: one byte stream, one JSON
//! value per line, and the api's own shapes in both directions.

pub mod client;
pub mod frame;
pub mod server;
pub(crate) mod stream;

use std::hash::{DefaultHasher, Hash, Hasher};
use std::path::{Path, PathBuf};

use crate::server::api::{Call, Opened, Post, PostDone, Reply, StreamItem, Subscribe, Viewer};

/// One line from a front end. Each door's payload is a named field, so every
/// shape of every door stays serializable.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Ask {
    /// I am here, and this is the id I go by.
    Hello { viewer: Viewer },
    /// I am leaving. A front end that dies without saying so is noticed all
    /// the same: its connection ends.
    Bye,
    /// One question, one answer.
    Call { call: Call },
    /// One change of session state, no answer.
    Post { post: Post },
    /// Watch a session, or open a stretch on one.
    Subscribe { subscribe: Subscribe },
}

/// One line from the server.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum Answer {
    Reply { reply: Reply },
    Done { done: PostDone },
    Opened { opened: Opened },
    Stream { item: StreamItem },
    /// The door failed. The connection stays open; the next line is welcome.
    Error { message: String },
}

impl Answer {
    /// The line a door sends back when it failed.
    pub fn error(e: &anyhow::Error) -> Answer {
        Answer::Error {
            message: format!("{e:#}"),
        }
    }
}

/// Where this database's server listens: one socket name per database.
pub fn socket_path(db: &Path) -> PathBuf {
    crate::server::platform::env::runtime_dir().join(format!("mypi-{}.sock", fingerprint(db)))
}

/// A short, stable name for one database path.
fn fingerprint(db: &Path) -> String {
    let mut hasher = DefaultHasher::new();
    db.hash(&mut hasher);
    format!("{:012x}", hasher.finish())
}

/// A front end's own id: this process, plus a counter inside it.
pub fn viewer() -> Viewer {
    static NEXT: std::sync::atomic::AtomicI64 = std::sync::atomic::AtomicI64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    Viewer(((std::process::id() as i64) << 20) | (seq & 0xf_ffff))
}
