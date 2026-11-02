//! The handover memo: aliases and their content, owned by the app.

use std::cell::RefCell;
use std::collections::HashMap;

/// What one zone leaves for the next. The app stores it without reading it.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Memo {
    slots: RefCell<HashMap<String, String>>,
}

impl Memo {
    /// Overwrite one alias.
    pub fn write(&self, alias: &str, content: &str) {
        self.slots
            .borrow_mut()
            .insert(alias.to_string(), content.to_string());
    }

    /// One alias, if it is there.
    pub fn read(&self, alias: &str) -> Option<String> {
        self.slots.borrow().get(alias).cloned()
    }

    /// Drop one alias.
    pub fn delete(&self, alias: &str) {
        self.slots.borrow_mut().remove(alias);
    }

    pub fn clear(&mut self) {
        self.slots.get_mut().clear();
    }
}
