//! `call.config`: one registry-shaped config value.

/// One config value.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConfigValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    Text(String),
    List(Vec<ConfigValue>),
}

/// Request: `{key}`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Ask {
    pub key: String,
}

/// Reply: `{value}`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Reply {
    pub value: ConfigValue,
}
