//! What the input zone's parts read: its own data, and the terminal it draws into.

use std::time::Duration;

use super::super::num;
use crate::ui::lang::Lang;

/// The currency the cost pill counts in; the config answers, `CNY` until it does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Currency {
    #[default]
    Cny,
    Usd,
}

impl Currency {
    /// The config's spelling, `CNY` or `USD`; anything else leaves the choice alone.
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "cny" | "rmb" => Some(Self::Cny),
            "usd" => Some(Self::Usd),
            _ => None,
        }
    }

    pub fn symbol(self) -> &'static str {
        match self {
            Self::Cny => "¥",
            Self::Usd => "$",
        }
    }
}

/// What the session has cost, both sides of the rate; the display picks one.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Money {
    pub cny: f64,
    pub usd: f64,
    /// The subagents' pool in the display currency; `None` = no price known or all zero.
    pub sub: Option<f64>,
    pub currency: Currency,
}

impl Money {
    /// The pill's text: `¥1.23(+0.45)` while the subagents' pool carries a price, `¥1.23` alone
    /// when it does not; `¥0.00` until the first price is known.
    pub fn text(&self) -> String {
        let total = match self.currency {
            Currency::Cny => self.cny,
            Currency::Usd => self.usd,
        };
        let mut out = num::money(total, self.currency.symbol());
        if let Some(sub) = self.sub.filter(|sub| *sub > 0.0) {
            // The same currency and precision as the pill itself, without the symbol.
            out.push_str(&format!("(+{})", num::money(sub, "")));
        }
        out
    }
}

/// The context the next request would carry, and whether that number is an estimate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Context {
    pub tokens: Option<u64>,
    pub estimated: bool,
}

/// Everything the editor, the statusline and the desk's hands read.
#[derive(Debug)]
pub struct Data {
    /// The model's display name; `None` while the config has not answered.
    pub model: Option<String>,
    /// The model's provider, once the config entry has answered.
    pub provider: Option<String>,
    /// The display language, from `config.ui.lang`.
    pub lang: Lang,
    /// The model's address, `<provider>:<model>`.
    pub address: Option<String>,
    /// How hard this session asks the model to think; memory-only, `Auto` by default.
    pub effort: String,
    /// The rungs the model declares; empty means it takes no effort.
    pub efforts: Vec<String>,
    /// The conversation's name; a draft session shows its directory instead.
    pub name: Option<String>,
    /// Where the conversation runs.
    pub cwd: String,
    /// The directory moved for one tool call only.
    pub temporary: bool,
    /// Where the context stands; the model's window is `limit`.
    pub context: Context,
    /// The archive's smallest measured context: what to show before a request lands.
    pub baseline: Option<u64>,
    /// The model's context window.
    pub limit: u64,
    /// What the session has cost so far.
    pub money: Money,
    /// Seconds between git scans of the workspace; `None` until the config answers.
    pub git_scan: Option<Duration>,
    /// The first message of a session, and the first after a compaction, carry the preparation.
    pub auto_prepare: bool,
    /// A round is running.
    pub busy: bool,
}

impl Default for Data {
    fn default() -> Self {
        Data {
            model: None,
            provider: None,
            lang: Lang::default(),
            address: None,
            effort: "auto".to_string(),
            efforts: Vec::new(),
            name: None,
            cwd: String::new(),
            temporary: false,
            context: Context::default(),
            baseline: None,
            limit: 0,
            money: Money::default(),
            git_scan: None,
            auto_prepare: false,
            busy: false,
        }
    }
}

impl Data {
    /// Whether the model this zone shows declares a thinking effort of its own.
    pub fn takes_effort(&self) -> bool {
        !self.efforts.is_empty()
    }
}

/// What one part is handed: the zone's data.
pub struct Ctx<'a> {
    pub data: &'a Data,
}
