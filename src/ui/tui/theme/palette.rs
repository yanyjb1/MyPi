//! The one live accent, shared: a rename repaints the whole front end at once.

use std::cell::Cell;
use std::rc::Rc;

use super::Accent;

/// One color for every zone. Each holds a clone; there is only ever one value.
#[derive(Debug, Clone)]
pub struct Palette {
    live: Rc<Cell<Accent>>,
}

impl Default for Palette {
    fn default() -> Self {
        Palette {
            live: Rc::new(Cell::new(Accent::pick())),
        }
    }
}

impl Palette {
    pub fn get(&self) -> Accent {
        self.live.get()
    }

    /// A rename moves the whole screen to another entry of the pool.
    pub fn shift(&self) {
        self.live.set(self.live.get().pick_other());
    }
}
