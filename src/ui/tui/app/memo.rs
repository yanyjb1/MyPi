//! The handover memo: aliases and content, open only during handover/takeover.

use std::cell::RefCell;
use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Operator {
    Write,
    Read,
    Delete,
}

#[derive(Debug, Default)]
pub struct Memo {
    slots: RefCell<HashMap<String, String>>,
}

impl Memo {
    /// Write, read, or delete one alias. The caller owns the content's meaning.
    pub fn call(&self, op: Operator, content: &str, alias: &str) -> Option<String> {
        let mut slots = self.slots.borrow_mut();
        match op {
            Operator::Write => {
                slots.insert(alias.to_string(), content.to_string());
                None
            }
            Operator::Read => slots.get(alias).cloned(),
            Operator::Delete => slots.remove(alias),
        }
    }

    pub fn clear(&mut self) {
        self.slots.get_mut().clear();
    }
}
