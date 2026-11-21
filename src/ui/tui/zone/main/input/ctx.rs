// The figures the statusline and the editor read: one row, owned by the id this zone follows.

use std::time::Duration;

use super::super::num;
use crate::ui::tui::lang::Lang;

// The id of a draft. No session exists yet, so the row belongs to an id the server never mints.
pub const DRAFT: i64 = -1;

// The currency the cost pill counts in; the config answers, CNY until it does.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Currency {
    #[default]
    Cny,
    Usd,
}

impl Currency {
    // The config's spelling, CNY or USD; anything else leaves the choice alone.
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

// What the session has cost, both sides of the rate; the display picks one.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Money {
    pub cny: f64,
    pub usd: f64,
    // The subagents' pool in the display currency; `None` = no price known or all zero.
    pub sub: Option<f64>,
    pub currency: Currency,
}

impl Money {
    // The pill's text: `¥1.23(+0.45)` while the subagents' pool carries a price, `¥1.23` alone when it does not; `¥0.00` until the first price is known.
    pub fn text(&self) -> String {
        let total = match self.currency {
            Currency::Cny => self.cny,
            Currency::Usd => self.usd,
        };
        let mut out = num::money(total, self.currency.symbol());
        if let Some(sub) = self.sub.filter(|sub| *sub > 0.0) {
            out.push_str(&format!("(+{})", num::money(sub, "")));
        }
        out
    }
}

// The context the next request would carry, and whether that number is an estimate.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Context {
    pub tokens: Option<u64>,
    pub estimated: bool,
}

// Everything the editor, the statusline and the desk's hands read of one id.
#[derive(Debug, Clone, Default)]
pub struct Data {
    // The session these figures describe; `DRAFT` until a session exists.
    pub key: i64,

    // The model's display name; `None` while the config has not answered.
    pub model: Option<String>,

    // The model's provider, once the config entry has answered.
    pub provider: Option<String>,

    // The display language, from `config.ui.lang`.
    pub lang: Lang,

    // The model's address, `<provider>:<model>`.
    pub address: Option<String>,

    // How hard this id asks the model to think; memory-only, `Auto` by default.
    pub effort: String,

    // The rungs the model declares; empty means it takes no effort.
    pub efforts: Vec<String>,

    // The conversation's name; a draft shows its directory instead.
    pub name: Option<String>,

    // Where the conversation runs.
    pub cwd: String,

    // The directory moved for one tool call only.
    pub temporary: bool,

    // Where the context stands; the model's window is `limit`.
    pub context: Context,

    // The archive's smallest measured context: what to show before a request lands.
    pub baseline: Option<u64>,

    // The model's context window.
    pub limit: u64,

    // What this id has cost so far.
    pub money: Money,

    // Seconds between git scans of the workspace; `None` until the config answers.
    pub git_scan: Option<Duration>,

    // The first message of a session, and the first after a compaction, carry the preparation.
    pub auto_prepare: bool,

    // A round is running.
    pub busy: bool,
}

impl Data {
    // Whether the model this id shows declares a thinking effort of its own.
    pub fn takes_effort(&self) -> bool {
        !self.efforts.is_empty()
    }

    // Point the row at another id. Its figures are the new id's to answer, so they go; the language, the directory, the scan interval, the preparation switch and the currency are the process's and stay.
    pub fn turn(&mut self, key: i64) {
        if key == self.key {
            return;
        }
        let lang = self.lang;
        let cwd = self.cwd.clone();
        let git_scan = self.git_scan;
        let auto_prepare = self.auto_prepare;
        let currency = self.money.currency;
        *self = Data {
            key,
            lang,
            cwd,
            git_scan,
            auto_prepare,
            money: Money {
                currency,
                ..Money::default()
            },
            ..Data::default()
        };
    }
}

// What one part is handed: the followed id's row.
pub struct Ctx<'a> {
    pub data: &'a Data,
}
