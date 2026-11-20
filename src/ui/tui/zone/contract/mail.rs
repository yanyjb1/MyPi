// The post box: every zone drops its letters here, the front end empties it in one go.

use std::cell::RefCell;
use std::rc::Rc;

use crate::ipc::contract::Frame;

use super::outcome::ZoneId;

// One box, held by every zone and by the front end. Nothing here reads a letter.
#[derive(Debug, Clone, Default)]
pub struct Mail {
    queue: Rc<RefCell<Vec<(ZoneId, Frame)>>>,
    // Who this handle posts for.
    from: ZoneId,
}

impl Mail {
    // The handle one zone posts with: every letter it sends names it as the sender.
    pub fn at(&self, from: ZoneId) -> Mail {
        Mail {
            queue: Rc::clone(&self.queue),
            from,
        }
    }

    pub fn send(&self, frame: Frame) {
        self.queue.borrow_mut().push((self.from, frame));
    }

    // Everything posted since the last call, in posting order, with the poster of each.
    pub fn drain(&self) -> Vec<(ZoneId, Frame)> {
        std::mem::take(&mut *self.queue.borrow_mut())
    }
}
