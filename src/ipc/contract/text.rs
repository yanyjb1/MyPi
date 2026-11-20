// One user-visible line: a kind with a machine key and optional arguments, or an upstream body.

use serde::{Deserialize, Serialize};
use serde_json::Value;

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
