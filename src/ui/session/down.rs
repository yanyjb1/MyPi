// The downstream half: the frames a front end reads, and the structural window it borrows.

use crate::ipc::contract::Frame;

use super::structure::Structure;

#[derive(Default)]
pub struct Down {
    // Arrival order, since the last handover.
    outbox: Vec<Frame>,
    structure: Structure,
}

impl Down {
    // A frame the session has taken off the wire.
    pub fn push(&mut self, frame: Frame) {
        self.outbox.push(frame);
    }

    pub fn receive(&mut self) -> Vec<Frame> {
        std::mem::take(&mut self.outbox)
    }

    pub fn structure(&self) -> &Structure {
        &self.structure
    }

    pub fn structure_mut(&mut self) -> &mut Structure {
        &mut self.structure
    }
}
