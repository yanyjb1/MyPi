//! The two name pools completion draws from, decoded from `call pools`.

use serde_json::Value;

use crate::ui::command::Candidate;

/// Model addresses and profile names, ready for the completion engine.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct Pools {
    pub models: Vec<Candidate>,
    pub profiles: Vec<Candidate>,
}

impl Pools {
    /// Decode the `pools` answer.
    pub fn of(answer: &Value) -> Pools {
        let models = answer
            .get("models")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(|row| {
                        let value = row.get("address")?.as_str()?.to_string();
                        let name = row.get("name").and_then(Value::as_str).unwrap_or("");
                        let id = row.get("id").and_then(Value::as_str).unwrap_or("");
                        Some(Candidate {
                            // The row is read for its name; the id is only a fallback.
                            detail: if name.is_empty() {
                                id.to_string()
                            } else {
                                name.to_string()
                            },
                            value,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
        let profiles = answer
            .get("profiles")
            .and_then(Value::as_array)
            .map(|rows| {
                rows.iter()
                    .filter_map(Value::as_str)
                    .map(|name| Candidate {
                        value: name.to_string(),
                        detail: String::new(),
                    })
                    .collect()
            })
            .unwrap_or_default();
        Pools { models, profiles }
    }
}
