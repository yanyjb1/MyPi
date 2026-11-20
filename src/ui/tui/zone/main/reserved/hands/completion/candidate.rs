//! One completion candidate: the text a level inserts, and the gray text beside it.

/// One completion candidate for one level.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Candidate {
    /// The text this candidate inserts for this level.
    pub value: String,

    /// What the popup shows next to the value.
    pub detail: String,
}
