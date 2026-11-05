//! What the input zone's parts read: its own data, and the terminal it draws into.

use super::super::num;

/// What the session has cost: the stored total, plus the running turn priced locally.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Money {
    /// Everything the database already accounts for.
    pub base: f64,
    /// This turn's bills, counted since the last base.
    pub live: f64,
    pub symbol: String,
}

impl Money {
    /// The pill's text: always one, `¥0.00` until the first price is known.
    pub fn text(&self) -> String {
        let total = self.base + self.live;
        let symbol = if self.symbol.is_empty() {
            "¥"
        } else {
            self.symbol.as_str()
        };
        num::money(total, symbol)
    }
}

/// One model's unit prices, per million tokens.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Prices {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

impl Prices {
    /// What one request cost at these prices.
    pub fn bill(&self, bill: &Bill) -> f64 {
        bill.prompt as f64 / 1_000_000.0 * self.input
            + bill.completion as f64 / 1_000_000.0 * self.output
            + bill.cached as f64 / 1_000_000.0 * self.cache_read
            + bill.cache_write as f64 / 1_000_000.0 * self.cache_write
    }
}

/// One request's four counts, as the loop wrote them.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Bill {
    pub prompt: u64,
    pub completion: u64,
    pub cached: u64,
    pub cache_write: u64,
}

/// Everything the editor, the statusline and the desk's hands read.
#[derive(Debug, Default)]
pub struct Data {
    /// The model's display name; `None` while the config has not answered.
    pub model: Option<String>,
    /// The model's address, `<provider>:<model>`.
    pub address: Option<String>,
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
    /// The running model's unit prices; `None` until the config answers.
    pub prices: Option<Prices>,
    /// A round is running.
    pub busy: bool,
    /// A conversation is open; a profile can no longer be chosen.
    pub attached: bool,
    /// One line about the last command; an edit clears it.
    pub hint: Option<String>,
}

/// What one part is handed: the zone's data.
pub struct Ctx<'a> {
    pub data: &'a Data,
}
