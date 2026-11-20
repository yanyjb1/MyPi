// The compiled-in display text: one JSON file per language, looked up by a caller-chosen key.

use std::collections::HashMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use serde_json::Value;

// The language this binary falls back to.
const DEFAULT_LANGUAGE: &str = "en";

// Every language embedded into the binary: code and its JSON text.
const LANGUAGES: &[(&str, &str)] = &[
    ("en", include_str!("en.json")),
    ("zh", include_str!("zh.json")),
];

const NOT_FOUND: &str = "not found";

// What a piece of text is for: an error, a warning, a notice, or an upstream message.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    Error,
    Warning,
    Notice,
    Upstream,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Notice => "notice",
            Self::Upstream => "upstream",
        }
    }
}

static TABLES: OnceLock<HashMap<&'static str, HashMap<String, String>>> = OnceLock::new();

// The sentence template for a key, if this language and key have one.
pub fn text(lang: &str, key: &str) -> Option<&'static str> {
    tables().get(resolve(lang))?.get(key).map(String::as_str)
}

// One sentence: the language's template with `{name}` filled from `args`.
pub fn render(lang: &str, key: &str, args: &Value) -> String {
    match text(lang, key) {
        Some(template) => fill(template, args),
        None => NOT_FOUND.to_string(),
    }
}

// The language this binary falls back to when the asked-for one has no file.
pub fn resolve(lang: &str) -> &'static str {
    LANGUAGES
        .iter()
        .map(|(code, _)| *code)
        .find(|code| *code == lang)
        .unwrap_or(DEFAULT_LANGUAGE)
}

// The language codes compiled into this binary.
pub fn languages() -> impl Iterator<Item = &'static str> {
    LANGUAGES.iter().map(|(code, _)| *code)
}

fn tables() -> &'static HashMap<&'static str, HashMap<String, String>> {
    TABLES.get_or_init(|| {
        LANGUAGES
            .iter()
            .map(|(code, text)| {
                let table: HashMap<String, String> = serde_json::from_str(text)
                    .expect("a language file is a JSON object of strings");
                (*code, table)
            })
            .collect()
    })
}

fn fill(template: &str, args: &Value) -> String {
    let mut out = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let tail = &rest[start + 1..];
        let Some(end) = tail.find('}') else {
            out.push_str(&rest[start..]);
            return out;
        };
        let name = &tail[..end];
        match args.get(name) {
            Some(Value::String(text)) => out.push_str(text),
            Some(Value::Number(number)) => out.push_str(&number.to_string()),
            Some(Value::Bool(true)) => out.push_str("true"),
            Some(Value::Bool(false)) => out.push_str("false"),
            Some(Value::Array(items)) => {
                for (index, item) in items.iter().enumerate() {
                    if index > 0 {
                        out.push_str(", ");
                    }
                    out.push_str(&text_of(item));
                }
            }
            Some(other) => out.push_str(&text_of(other)),
            None => {
                out.push('{');
                out.push_str(name);
                out.push('}');
            }
        }
        rest = &tail[end + 1..];
    }
    out.push_str(rest);
    out
}

fn text_of(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        other => other.to_string(),
    }
}
