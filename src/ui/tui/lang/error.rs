// One line the server sent: this language's rendering of its key, or its own words.

use serde_json::Value;

use super::Lang;

// The sentence inside an error payload; `None` when the answer carries no error.
pub fn refused(lang: Lang, payload: &Value) -> Option<String> {
    payload.get("error").map(|error| sentence(lang, error))
}

// One text value: a key rendered through i18n, or an upstream body passed through.
pub fn sentence(lang: Lang, value: &Value) -> String {
    match value.get("key").and_then(Value::as_str) {
        Some(key) => {
            let args = value.get("args").cloned().unwrap_or(Value::Null);
            crate::i18n::render(lang.code(), key, &args)
        }
        None => value
            .get("text")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
    }
}
