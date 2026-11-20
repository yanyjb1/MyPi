// The compiled-in display text: one JSON file per language, looked up by a caller-chosen key.

use std::collections::HashMap;
use std::sync::OnceLock;

// The language this binary falls back to.
const DEFAULT_LANGUAGE: &str = "en";

// Every language embedded into the binary: code and its JSON text.
const LANGUAGES: &[(&str, &str)] = &[
    ("en", include_str!("en.json")),
    ("zh", include_str!("zh.json")),
];

const NOT_FOUND: &str = "not found";

// What a piece of text is for: an error, a warning, or a notice.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Kind {
    Error,
    Warning,
    Notice,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Error => "error",
            Self::Warning => "warning",
            Self::Notice => "notice",
        }
    }
}

static TABLES: OnceLock<HashMap<&'static str, HashMap<String, String>>> = OnceLock::new();

// The sentence for a key, or `not found` when the language or the key has none.
pub fn text(lang: &str, key: &str) -> &'static str {
    tables()
        .get(resolve(lang))
        .and_then(|table| table.get(key))
        .map(String::as_str)
        .unwrap_or(NOT_FOUND)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_key_reads_in_both_languages() {
        assert_eq!(text("en", "busy"), "the session is already in a turn");
        assert_eq!(text("zh", "busy"), "会话正在跑一轮");
    }

    #[test]
    fn a_missing_key_is_not_found() {
        assert_eq!(text("en", "no_such_key"), "not found");
    }

    #[test]
    fn a_missing_language_falls_back_to_english() {
        assert_eq!(text("xx", "busy"), text("en", "busy"));
    }

    #[test]
    fn the_default_language_is_compiled_in() {
        assert!(languages().any(|code| code == DEFAULT_LANGUAGE));
    }
}
