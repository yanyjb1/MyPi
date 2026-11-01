//! One sub-session's live state: the draft it edits, its budget, and where its answer lands.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use anyhow::{Result, bail};

use crate::server::agent::tools::contract::view::{Child, Draft};

/// How many times a child is told to finish before its task counts as failed.
const NAGS: u32 = 3;

/// What a child is told when a round ends without a submit.
const NAG: &str = "You have not finished. Put your answer in the draft and call `final submit`; \
                   nothing else ends your task.";

/// A sub-session's state. It dies with the session.
pub struct State {
    budget: Mutex<u32>,
    draft: Mutex<String>,
    answer: PathBuf,
    submitted: AtomicBool,
    nags: AtomicU32,
}

impl State {
    pub fn child(answer: PathBuf, budget: u32) -> Self {
        Self {
            budget: Mutex::new(budget),
            draft: Mutex::new(String::new()),
            answer,
            submitted: AtomicBool::new(false),
            nags: AtomicU32::new(0),
        }
    }
}

impl Child for State {
    fn draft(&self) -> &dyn Draft {
        self
    }

    fn left(&self) -> u32 {
        *self.budget.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn charge(&self) -> Result<()> {
        let mut left = self.budget.lock().unwrap_or_else(|e| e.into_inner());
        if *left == 0 {
            bail!("tool-call budget exhausted");
        }
        *left -= 1;
        Ok(())
    }

    fn answer(&self) -> &Path {
        &self.answer
    }

    fn submit(&self) {
        self.submitted.store(true, Ordering::Release);
    }

    fn submitted(&self) -> bool {
        self.submitted.load(Ordering::Acquire)
    }

    fn nag(&self) -> Option<&'static str> {
        if self.submitted.load(Ordering::Acquire) {
            return None;
        }
        self.nags
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                (n < NAGS).then_some(n + 1)
            })
            .ok()
            .map(|_| NAG)
    }
}

impl Draft for State {
    fn read(&self) -> Result<String> {
        Ok(self.draft.lock().unwrap_or_else(|e| e.into_inner()).clone())
    }

    fn write(&self, text: &str) -> Result<()> {
        *self.draft.lock().unwrap_or_else(|e| e.into_inner()) = text.to_string();
        Ok(())
    }
}
