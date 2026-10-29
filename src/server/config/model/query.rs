use super::super::registry::{self, Value};
use super::types::{ModelEntry, ModelsConfig, Provider, ResolvedModel};

impl ModelsConfig {
    pub fn provider(&self, name: &str) -> Option<&Provider> {
        self.providers.iter().find(|p| p.name == name)
    }

    pub fn models(&self) -> impl Iterator<Item = (&str, &ModelEntry)> {
        self.providers
            .iter()
            .flat_map(|p| p.models.iter().map(move |m| (p.name.as_str(), m)))
    }

    pub fn model_by_id(&self, address: &str) -> Option<ResolvedModel> {
        let (provider, id) = address.split_once(':')?;
        let p = self.provider(provider)?;
        let entry = p.models.iter().find(|m| m.id == id).cloned()?;
        Some(ResolvedModel {
            provider_name: p.name.clone(),
            entry,
        })
    }

    pub fn default_model(&self, configured: Option<&str>) -> Option<ResolvedModel> {
        match configured {
            Some(address) => self.model_by_id(address),
            None => self.alphabetical_default(),
        }
    }

    pub fn alphabetical_default(&self) -> Option<ResolvedModel> {
        self.models()
            .min_by_key(|(provider, model)| {
                (
                    model.display_name().to_lowercase(),
                    (*provider).to_string(),
                    model.id.clone(),
                )
            })
            .and_then(|(provider, model)| {
                Some(ResolvedModel {
                    provider_name: provider.to_string(),
                    entry: (*model).clone(),
                })
            })
    }

    pub fn resolve_key(&self, provider: &Provider) -> String {
        let raw = provider.api_key.trim();
        if let Some(inner) = raw.strip_prefix("${").and_then(|r| r.strip_suffix('}')) {
            crate::server::platform::env::var(inner).unwrap_or_default()
        } else {
            raw.to_string()
        }
    }

    pub fn get(&self, key: &str) -> Result<Value, String> {
        if let Some(rest) = key.strip_prefix("models.provider.") {
            let (name, field) = rest
                .split_once('.')
                .ok_or_else(|| "model provider key needs a field".to_string())?;
            let provider = self
                .provider(name)
                .ok_or_else(|| format!("unknown provider: {name}"))?;
            return match field {
                "baseUrl" => Ok(Value::Text(provider.base_url.clone())),
                "api" => Ok(Value::Text(provider.api.clone())),
                "authHeader" => Ok(Value::Bool(provider.auth_header)),
                "models" => Ok(Value::List(
                    provider.models.iter().map(|m| m.id.clone()).collect(),
                )),
                "apiKey" => Err("apiKey is not readable through the key API".into()),
                other => Err(format!("unknown provider field: {other}")),
            };
        }
        let rest = key
            .strip_prefix("models.model.")
            .ok_or_else(|| format!("unknown key: {key}"))?;
        for suffix in registry::model_suffixes() {
            let tail = format!(".{suffix}");
            let Some(address) = rest.strip_suffix(&tail) else {
                continue;
            };
            let model = self
                .model_by_id(address)
                .ok_or_else(|| format!("unknown model: {address}"))?;
            let entry = &model.entry;
            return match suffix.as_str() {
                "id" => Ok(Value::Text(entry.id.clone())),
                "name" => Ok(Value::Text(entry.display_name().to_string())),
                "currency" => Ok(Value::Text(entry.currency.as_str().to_string())),
                "contextWindow" => Ok(optional_int(entry.context_window)),
                "maxOutputTokens" => Ok(optional_int(entry.max_output_tokens)),
                "cost.input" => Ok(Value::Float(entry.cost.input)),
                "cost.output" => Ok(Value::Float(entry.cost.output)),
                "cost.cacheRead" => Ok(Value::Float(entry.cost.cache_read)),
                "cost.cacheWrite" => Ok(Value::Float(entry.cost.cache_write)),
                _ => Err(format!("unknown model field: {suffix}")),
            };
        }
        Err(format!("unknown model field: {key}"))
    }
}

fn optional_int(value: Option<u64>) -> Value {
    match value {
        Some(n) if n <= i64::MAX as u64 => Value::Int(n as i64),
        Some(_) => Value::Int(i64::MAX),
        None => Value::Null,
    }
}

pub fn is_model_key(key: &str) -> bool {
    key.starts_with("models.")
}
