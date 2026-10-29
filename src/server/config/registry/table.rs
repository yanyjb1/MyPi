use super::super::schema::KEYS;
use super::types::{BoolOrInt, Key, Value};

pub fn lookup(full: &str) -> Option<&'static Key> {
    KEYS.iter().find(|k| k.full == full && k.queryable)
}

pub fn is_config_key(full: &str) -> bool {
    KEYS.iter().any(|k| k.head == "config" && k.full == full)
}

pub fn is_config_prefix(prefix: &str) -> bool {
    KEYS.iter().any(|k| {
        k.head == "config"
            && k.full.len() > prefix.len()
            && k.full.starts_with(prefix)
            && k.full.as_bytes().get(prefix.len()) == Some(&b'.')
    })
}

pub fn config_keys() -> impl Iterator<Item = &'static Key> {
    KEYS.iter().filter(|k| k.head == "config")
}

pub fn provider_key(provider: &str, field: &str) -> String {
    format!("models.provider.{provider}.{field}")
}

pub fn model_key(address: &str, field: &str) -> String {
    format!("models.model.{address}.{field}")
}

pub fn template() -> String {
    let mut out = String::from(
        "# mypi configuration — every key with its default.\n\
         # Edit what you need; the rest keeps the default.\n\
         config:\n",
    );
    let mut open: Vec<&str> = Vec::new();
    for key in config_keys() {
        let segs: Vec<&str> = if key.path.is_empty() {
            Vec::new()
        } else {
            key.path.split('.').collect()
        };
        let common = open.iter().zip(&segs).take_while(|(a, b)| a == b).count();
        open.truncate(common);
        for (i, seg) in segs.iter().enumerate().skip(common) {
            out.push_str(&format!("{}{}:\n", "  ".repeat(i + 1), seg));
            open.push(*seg);
        }
        let indent = "  ".repeat(segs.len() + 1);
        out.push_str(&format!("{indent}# {}\n", key.comment));
        out.push_str(&format!(
            "{indent}{}: {}\n",
            key.name,
            yaml_scalar(&key.default_value())
        ));
    }
    out
}

fn yaml_scalar(value: &Value) -> String {
    match value {
        Value::Bool(b) => b.to_string(),
        Value::Int(i) => i.to_string(),
        Value::Float(f) if f.fract() == 0.0 => format!("{f:.1}"),
        Value::Float(f) => f.to_string(),
        Value::Text(s) => serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into()),
        Value::List(items) => {
            let body = items
                .iter()
                .map(|s| serde_json::to_string(s).unwrap_or_else(|_| "\"\"".into()))
                .collect::<Vec<_>>()
                .join(", ");
            format!("[{body}]")
        }
        Value::BoolOrInt(BoolOrInt::Bool(b)) => b.to_string(),
        Value::BoolOrInt(BoolOrInt::Int(i)) => i.to_string(),
        Value::Null => "null".into(),
    }
}

const MODEL_PREFIX: &str = "model.<address>";

pub fn provider_fields() -> impl Iterator<Item = &'static str> {
    KEYS.iter()
        .filter(|k| k.head == "models" && k.path == "provider.<name>")
        .map(|k| k.name)
}

pub fn model_fields() -> impl Iterator<Item = &'static str> {
    KEYS.iter()
        .filter(|k| k.head == "models" && k.path.starts_with(MODEL_PREFIX))
        .map(|k| {
            if k.path == MODEL_PREFIX {
                k.name
            } else {
                k.path[MODEL_PREFIX.len() + 1..]
                    .split('.')
                    .next()
                    .unwrap_or("")
            }
        })
}

pub fn cost_fields() -> impl Iterator<Item = &'static str> {
    KEYS.iter()
        .filter(|k| k.head == "models" && k.path.starts_with("model.<address>.cost"))
        .map(|k| k.name)
}

pub fn model_suffixes() -> impl Iterator<Item = String> {
    KEYS.iter()
        .filter(|k| k.head == "models" && k.path.starts_with(MODEL_PREFIX))
        .map(|k| {
            if k.path == MODEL_PREFIX {
                k.name.to_string()
            } else {
                format!("{}.{}", &k.path[MODEL_PREFIX.len() + 1..], k.name)
            }
        })
}
