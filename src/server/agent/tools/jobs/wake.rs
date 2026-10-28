//! Wake messages and the sink the API implements.

use super::job::{Id, TaskState};

#[derive(Debug, Clone)]
pub struct Wake {
    pub(crate) session: i64,
    pub(crate) text: String,
}

pub trait WakeSink: Send + Sync {
    fn wake(&self, wake: Wake);
}

pub(crate) fn job_status(
    id: Id,
    state: TaskState,
    exit: Option<i32>,
    duration: u64,
    bytes: usize,
    truncated: bool,
) -> String {
    let mut text = format!("job {id} {state}");
    if let Some(code) = exit {
        text.push_str(&format!(", exit {code}"));
    }
    text.push_str(&format!(", {duration}s, {bytes} bytes"));
    if truncated {
        text.push_str(", truncated");
    }
    text
}

pub(crate) fn skipped(id: Id, reason: &str, skipped: u32) -> String {
    format!("job {id} skipped ({reason}), {skipped} skipped so far")
}

pub(crate) fn canceled(id: Id) -> String {
    format!("job {id} canceled")
}
