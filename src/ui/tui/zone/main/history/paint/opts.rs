//! What painting needs besides the block itself.

/// Fold switches, the placeholder text, the working directory, the card budget.
pub struct Opts<'a> {
    pub thinking: bool,
    pub tool: bool,
    pub placeholder: &'a str,
    pub cwd: &'a str,
    pub fold: usize,
}
