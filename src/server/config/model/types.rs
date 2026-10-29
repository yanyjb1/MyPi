//! The `models.` types.

#[derive(Debug, Clone)]
pub struct ModelsConfig {
    pub providers: Vec<Provider>,
}

#[derive(Debug, Clone)]
pub struct Provider {
    pub name: String,
    pub base_url: String,
    pub api: String,
    pub api_key: String,
    pub auth_header: bool,
    pub models: Vec<ModelEntry>,
}

#[derive(Debug, Clone)]
pub struct ModelEntry {
    pub id: String,
    pub name: String,
    pub context_window: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub currency: Currency,
    pub cost: Cost,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Currency {
    #[default]
    CNY,
    USD,
}

impl Currency {
    pub fn as_str(self) -> &'static str {
        match self {
            Currency::CNY => "CNY",
            Currency::USD => "USD",
        }
    }

    pub(super) fn parse(raw: &str) -> Option<Self> {
        match raw {
            "CNY" => Some(Currency::CNY),
            "USD" => Some(Currency::USD),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Cost {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
}

#[derive(Debug, Clone)]
pub struct ResolvedModel {
    pub provider_name: String,
    pub entry: ModelEntry,
}

impl ModelEntry {
    pub fn display_name(&self) -> &str {
        if self.name.is_empty() {
            &self.id
        } else {
            &self.name
        }
    }
}
