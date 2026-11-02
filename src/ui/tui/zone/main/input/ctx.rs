//! What the input zone's parts read: its own data, and the terminal it draws into.

use super::super::num;

/// What the session has cost, in the currency the price table names.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Money {
    pub total: f64,
    pub symbol: String,
}

impl Money {
    /// The row's own text, or nothing when there is no price to show.
    pub fn text(&self) -> Option<String> {
        (self.total > 0.0 && !self.symbol.is_empty()).then(|| num::money(self.total, &self.symbol))
    }
}

/// Everything the editor, the statusline and the desk's hands read.
#[derive(Debug, Default)]
pub struct Data {
    /// The conversation's model, as the session reports it.
    pub model: Option<String>,
    /// The conversation's name; a draft session shows its directory instead.
    pub name: Option<String>,
    /// Where the conversation runs.
    pub cwd: String,
    /// The directory moved for one tool call only.
    pub temporary: bool,
    /// The last request's context size, and the model's window.
    pub used: u64,
    pub limit: u64,
    /// What the session has cost so far.
    pub money: Money,
    /// A round is running.
    pub busy: bool,
}

/// What one part is handed: the zone's data.
pub struct Ctx<'a> {
    pub data: &'a Data,
}
