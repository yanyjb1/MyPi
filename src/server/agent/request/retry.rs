use std::time::Duration;

use crate::server::agent::request::message::NoticeLevel;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_retries: u32,
    pub interval: Duration,
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
    NoResponse { source: String },
    Http { status: u16, body: String },
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
pub enum RetryClass {
    NoResponse,
    TooManyRequests,
    Fatal,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetryNotice {
    pub level: NoticeLevel,
    pub reason: String,
    pub attempt: u32,
    pub max: u32,
}

pub fn classify(failure: &Failure, emitted: bool, cancelled: bool) -> RetryClass {
    if cancelled || emitted {
        return RetryClass::Fatal;
    }
    match failure {
        Failure::Cancelled => RetryClass::Fatal,
        Failure::NoResponse { .. } => RetryClass::NoResponse,
        Failure::Http { status: 429, .. } => RetryClass::TooManyRequests,
        Failure::Http { .. } => RetryClass::Fatal,
    }
}

pub fn notice(failure: &Failure, attempt: u32, max: u32, timeout: Duration) -> RetryNotice {
    match failure {
        Failure::Http { body, .. } => RetryNotice {
            level: NoticeLevel::Error,
            reason: body.clone(),
            attempt,
            max,
        },
        Failure::NoResponse { .. } => RetryNotice {
            level: NoticeLevel::Warn,
            reason: format!("request timeout: {timeout:?}"),
            attempt,
            max,
        },
        Failure::Cancelled => RetryNotice {
            level: NoticeLevel::Warn,
            reason: "cancelled".to_string(),
            attempt,
            max,
        },
    }
}
