//! The `models.` third of the configuration: what the user's `models.yml` says.

use serde::{Deserialize, Serialize};

/// Four unit prices per 1M tokens (USD): input, output, cache read, cache write. `cacheWrite` 0
/// means "price it as plain input".
#[derive(Debug, Clone, Copy, Deserialize, Serialize)]
pub struct Cost {
    #[serde(default)]
    pub input: f64,
    #[serde(default)]
    pub output: f64,
    #[serde(default, rename = "cacheRead")]
    pub cache_read: f64,
    #[serde(default, rename = "cacheWrite")]
    pub cache_write: f64,
}

impl Default for Cost {
    fn default() -> Self {
        Self {
            input: 0.0,
            output: 0.0,
            cache_read: 0.0,
            cache_write: 0.0,
        }
    }
}

/// Display currency of one model's price list: CNY (¥, the default), USD, or credits.
/// The rate that folds one into another lives in `config.yaml` (`fx.usdCny`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize, Serialize)]
pub enum Currency {
    #[default]
    Cny,
    Usd,
    Credits,
}

impl Currency {
    pub fn symbol(self) -> &'static str {
        match self {
            Currency::Usd => "$",
            Currency::Cny => "¥",
            Currency::Credits => "cr",
        }
    }
}

/// **models.yml** — the user's providers and models (credentials live here). The program
/// **never writes** this file.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct ModelsConfig {
    pub providers: std::collections::HashMap<String, Provider>,
}

/// Connection details for one provider: where it lives, the wire protocol, the key.
#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct Provider {
    #[serde(alias = "baseUrl")]
    pub base_url: String,
    /// Which wire protocol this provider speaks. It **decides the request
    /// assembly and the response parsing**: one provider, one protocol.
    #[serde(default = "default_api")]
    pub api: String,
    #[serde(default, alias = "apiKey")]
    pub api_key: String,
    /// Models this provider serves — **nested inside the provider**; the model id is
    /// provider-internal, so ids are written per provider by the user.
    #[serde(default, rename = "models")]
    pub models: Vec<ModelEntry>,
}

/// The default for the protocol key: the only one implemented.
pub fn default_api() -> String {
    crate::server::agent::request::protocols::ProtocolId::OpenAiCompletion
        .as_str()
        .to_string()
}

/// A protocol name as written in the config → the one protocol id inside the program. Only
/// chat/completions is recognized; the Responses API is out of scope.
pub fn normalize_api(name: &str) -> Option<&'static str> {
    match name.trim().to_ascii_lowercase().as_str() {
        "openai-completions" | "openai-chat-completions" | "openai" | "" => {
            Some(crate::server::agent::request::protocols::ProtocolId::OpenAiCompletion.as_str())
        }
        _ => None,
    }
}

/// A model plus the provider it lives under (the config nests models
/// inside providers; callers always need both).
#[derive(Debug, Clone)]
pub struct ResolvedModel {
    pub provider_name: String,
    pub entry: ModelEntry,
}

/// One callable model (a `- id: ...` entry in models.yml).
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct ModelEntry {
    /// Model id sent to the server ("vendor:name" form).
    pub id: String,
    /// Statusline display name (e.g. My Model).
    #[serde(default)]
    pub name: String,
    /// Context window (tokens); denominator of the statusline gauge.
    #[serde(default, rename = "contextWindow")]
    pub context_window: u64,
    /// User-capped output per reply — the actual `max_tokens` sent in
    #[serde(default, rename = "maxOutputTokens")]
    pub max_output_tokens: Option<u64>,
    #[serde(default)]
    pub currency: Currency,
    #[serde(default)]
    pub cost: Cost,
    /// This model's compatibility switches. **Absent = detect them from
    /// `baseUrl` + the model id**; written = the user's word wins.
    #[serde(default)]
    pub compat:
        Option<crate::server::agent::request::protocols::openai::completion::compat::Compat>,
}

impl ModelEntry {
    /// Human-facing name: the user-chosen `name` field, falling back to
    /// the wire id. Display only — the id is what the server sees.
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            &self.id
        } else {
            &self.name
        }
    }
}
