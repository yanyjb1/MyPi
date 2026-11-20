// One user-visible line: a kind with a machine key and optional arguments, or an upstream body.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::i18n::Kind;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Text {
    pub kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
}

impl Text {
    pub fn error(key: impl Into<String>) -> Text {
        Text::keyed(Kind::Error, key)
    }

    pub fn warning(key: impl Into<String>) -> Text {
        Text::keyed(Kind::Warning, key)
    }

    pub fn notice(key: impl Into<String>) -> Text {
        Text::keyed(Kind::Notice, key)
    }

    pub fn keyed(kind: Kind, key: impl Into<String>) -> Text {
        Text {
            kind,
            key: Some(key.into()),
            args: None,
            text: None,
        }
    }

    pub fn args(mut self, args: Value) -> Text {
        self.args = Some(args);
        self
    }

    pub fn upstream(text: impl Into<String>) -> Text {
        Text {
            kind: Kind::Upstream,
            key: None,
            args: None,
            text: Some(text.into()),
        }
    }
}
