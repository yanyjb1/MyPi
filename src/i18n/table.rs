// The language tables: which codes this binary carries, and how to reach one.

use std::collections::HashMap;
use std::sync::OnceLock;

// The language this binary falls back to.
const DEFAULT_LANGUAGE: &str = "en";

// Every language embedded into the binary: code and its JSON text.
const LANGUAGES: &[(&str, &str)] = &[
    ("en", include_str!("en.json")),
    ("zh", include_str!("zh.json")),
];

static TABLES: OnceLock<HashMap<&'static str, HashMap<String, String>>> = OnceLock::new();

// The sentence template for a key, if this language and key have one.
pub fn text(lang: &str, key: &str) -> Option<&'static str> {
    tables().get(resolve(lang))?.get(key).map(String::as_str)
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
