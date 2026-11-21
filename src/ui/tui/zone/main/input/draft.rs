// The draft: what a session will open with, kept by every zone until its row exists.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use crate::ui::tui::zone::Frame;

// One shared memo. Whoever needs it reads it; whoever changes it says so at once.
#[derive(Debug, Clone, Default)]
pub struct Draft {
    slots: Rc<RefCell<Slots>>,
}

#[derive(Debug, Clone, Default)]
struct Slots {
    cwd: PathBuf,
    name: Option<String>,
    model: Option<String>,
    profile: Option<String>,
    effort: Option<String>,
    // Whether the user picked the directory; a carried-over one lets the server fall back.
    picked: bool,
}

impl Draft {
    pub fn cwd(&self) -> PathBuf {
        self.slots.borrow().cwd.clone()
    }

    pub fn name(&self) -> Option<String> {
        self.slots.borrow().name.clone()
    }

    pub fn model(&self) -> Option<String> {
        self.slots.borrow().model.clone()
    }

    pub fn effort(&self) -> Option<String> {
        self.slots.borrow().effort.clone()
    }

    // The directory a user picked; the server may not move it behind their back.
    pub fn land(&self, cwd: PathBuf) {
        let mut slots = self.slots.borrow_mut();
        slots.cwd = cwd;
        slots.picked = true;
    }

    // The server named the directory it actually opened in.
    pub fn moved(&self, cwd: PathBuf) {
        self.slots.borrow_mut().cwd = cwd;
    }

    pub fn set_name(&self, name: Option<String>) {
        self.slots.borrow_mut().name = name;
    }

    pub fn set_model(&self, address: String) {
        self.slots.borrow_mut().model = Some(address);
    }

    pub fn set_profile(&self, profile: Option<String>) {
        self.slots.borrow_mut().profile = profile;
    }

    pub fn set_effort(&self, effort: String) {
        self.slots.borrow_mut().effort = Some(effort);
    }

    // The row an opening message needs.
    pub fn opening(&self) -> Frame {
        use serde_json::json;
        let slots = self.slots.borrow();
        let model = slots
            .model
            .as_deref()
            .and_then(|address| address.split_once(':'))
            .map(|(provider, model)| {
                json!({
                    "provider": provider,
                    "model": model,
                })
            });
        let content = json!({
            "cwd": slots.cwd.to_string_lossy(),
            "name": slots.name,
            "model": model,
            "profile": slots.profile,
            "effort": slots.effort,
            "fallback": !slots.picked,
        });
        Frame::new("post", "init", content)
    }

    // The row exists: the memos are spent, and the directory stays where it landed.
    pub fn clear(&self) {
        let cwd = self.cwd();
        *self.slots.borrow_mut() = Slots {
            cwd,
            ..Slots::default()
        };
    }
}
