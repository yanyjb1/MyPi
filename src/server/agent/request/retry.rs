//! Retry policy, failure classes, and the notice one failure produces.

use std::time::Duration;

use super::live::{Notice, NoticeLevel};

/// What one request is allowed to do before giving up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    /// Retries after the first attempt.
    pub max_retries: u32,
    /// The fixed wait between attempts.
    pub interval: Duration,
    /// How long one attempt may wait for the first byte.
    pub timeout: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_retries: 10,
            interval: Duration::from_secs(30),
            timeout: Duration::from_secs(120),
        }
    }
}

#[derive(Debug)]
pub enum Failure {
    /// Nothing arrived at all: a timeout, a refused connection, a dead worker.
    NoResponse { source: String },
    /// The host answered with a status outside 2xx.
    Http { status: u16, body: String },
    /// The user pressed escape.
    Cancelled,
}

impl std::fmt::Display for Failure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoResponse { source } => write!(f, "no response: {source}"),
            Self::Http { status, body } if body.trim().is_empty() => write!(f, "HTTP {status}"),
            Self::Http { status, body } => write!(f, "HTTP {status}: {}", body.trim()),
            Self::Cancelled => write!(f, "cancelled"),
        }
    }
}

impl std::error::Error for Failure {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Class {
    Retry,
    Fatal,
}

/// Only two things are worth another attempt: nothing arrived, or the host asked us to slow down.
pub fn classify(failure: &Failure, emitted: bool, cancelled: bool) -> Class {
    if cancelled || emitted {
        return Class::Fatal;
    }
    match failure {
        Failure::Cancelled => Class::Fatal,
        Failure::NoResponse { .. } => Class::Retry,
        Failure::Http { status: 429, .. } => Class::Retry,
        Failure::Http { .. } => Class::Fatal,
    }
}

/// The one line a watcher reads. A status error carries the host's body verbatim.
pub fn notice(failure: &Failure, timeout: Duration) -> Notice {
    match failure {
        Failure::Http { status, body } => Notice {
            level: NoticeLevel::Error,
            text: if body.trim().is_empty() {
                format!("HTTP {status}")
            } else {
                body.clone()
            },
        },
        Failure::NoResponse { .. } => Notice {
            level: NoticeLevel::Warn,
            text: format!("request timeout: {timeout:?}"),
        },
        Failure::Cancelled => Notice {
            level: NoticeLevel::Warn,
            text: "cancelled".to_string(),
        },
    }
}
