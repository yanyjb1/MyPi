//! `call.usage`: the usage bill for one session.

/// Number wording for the usage panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageLang {
    #[default]
    En,
    Zh,
}

/// The four billed cells.
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct UsageCost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

/// One usage row: one model's cumulative usage and bill within a session.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ModelUsage {
    pub id: String,
    pub provider: Option<String>,
    pub rounds: u64,
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub cached_tokens: u64,
    pub reasoning_tokens: u64,
    pub cache_write_tokens: u64,
    pub cost: Option<UsageCost>,
    pub currency: Option<String>,
}

/// Request: `{session, lang}`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Ask {
    pub session: i64,
    pub lang: UsageLang,
}

/// Reply: `{lang, groups}`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Reply {
    pub lang: UsageLang,
    pub groups: Vec<ModelUsage>,
}
