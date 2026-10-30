//! The tool-round fence.

/// Counts tool-call rounds, not individual calls.
pub struct Fence {
    max: Option<usize>,
    rounds: usize,
}

impl Fence {
    pub fn new(max: Option<usize>) -> Self {
        Self { max, rounds: 0 }
    }

    pub fn hit(&self) -> bool {
        self.max.is_some_and(|max| self.rounds >= max)
    }

    pub fn count(&mut self) {
        self.rounds += 1;
    }
}
