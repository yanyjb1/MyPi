//! One sub-session's live state: the draft it edits and what is left of its budget.

use std::sync::Mutex;

use anyhow::{Result, bail};

use crate::server::agent::tools::contract::{Child, Draft};

/// A sub-session's state. It dies with the session.
pub struct State {
    budget: Mutex<Option<u32>>,
    draft: Mutex<String>,
}

impl State {
    pub fn child(budget: u32) -> Self {
        Self {
            budget: Mutex::new(Some(budget)),
            draft: Mutex::new(String::new()),
        }
    }
}

impl Child for State {
    fn is_child(&self) -> bool {
        true
    }

    fn draft(&self) -> &dyn Draft {
        self
    }

    fn left(&self) -> Option<u32> {
        *self.budget.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn charge(&self) -> Result<()> {
        let mut budget = self.budget.lock().unwrap_or_else(|e| e.into_inner());
        match budget.as_mut() {
            Some(left) if *left > 0 => {
                *left -= 1;
                Ok(())
            }
            Some(_) => bail!("tool-call budget exhausted"),
            None => Ok(()),
        }
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
