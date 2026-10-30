//! The live line's sending end.

use std::sync::mpsc::Sender;

use super::element::{Element, Event, NoticeLevel, Part, ToolEvent};

/// One session's live line.
#[derive(Clone)]
pub struct Sink {
    tx: Sender<Event>,
    session: i64,
}

impl Sink {
    pub fn new(tx: Sender<Event>, session: i64) -> Self {
        Self { tx, session }
    }

    pub fn send(&self, element: Element) {
        let _ = self.tx.send(Event {
            session: self.session,
            element,
        });
    }

    pub fn increment(&self, part: Part, text: impl Into<String>) {
        self.send(Element::Increment {
            part,
            text: text.into(),
        });
    }

    pub fn boundary(&self) {
        self.send(Element::Boundary);
    }

    pub fn tool(&self, event: ToolEvent) {
        self.send(Element::Tool(event));
    }

    pub fn notice(&self, level: NoticeLevel, text: impl Into<String>) {
        self.send(Element::Notice {
            level,
            text: text.into(),
        });
    }
}
