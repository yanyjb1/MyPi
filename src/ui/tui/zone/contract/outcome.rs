//! Display right: who holds the screen, and what a zone says when it gives it up.

/// The three major zones. At most one holds the screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum ZoneId {
    #[default]
    Main,
    Resume,
    Tree,
}

impl ZoneId {
    /// Position in the registry.
    pub fn index(self) -> usize {
        match self {
            ZoneId::Main => 0,
            ZoneId::Resume => 1,
            ZoneId::Tree => 2,
        }
    }

    /// The zone a registry slot holds.
    pub fn of_index(at: usize) -> ZoneId {
        match at {
            0 => ZoneId::Main,
            1 => ZoneId::Resume,
            _ => ZoneId::Tree,
        }
    }
}

/// What a delivery made the screen do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The zone keeps the screen.
    Stay,

    /// The zone names its successor and leaves what it wants in the memo.
    Show {
        to: ZoneId,
        memo: Vec<(String, String)>,
    },

    /// Nobody shows; the loop ends.
    Quit,
}
