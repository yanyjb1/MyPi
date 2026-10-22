//! Where a logical line may be cut without losing the seam.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CutKind {
    /// The source had a newline here.
    Newline,
    /// A hard break introduced by the renderer.
    HardWrap,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CutPoint {
    pub line: usize,
    pub kind: CutKind,
}
