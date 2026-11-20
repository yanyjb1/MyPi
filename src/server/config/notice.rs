//! Configuration warnings: a key that was written but not used.

use serde_json::Value;

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigNotice {
    pub key: String,
    pub file: String,
    pub problem: String,
    pub used: String,
    pub args: Option<Value>,
}

pub(crate) fn warn(key: &str, file: &str, problem: &str, used: &str) -> ConfigNotice {
    ConfigNotice {
        key: key.to_string(),
        file: file.to_string(),
        problem: problem.to_string(),
        used: used.to_string(),
        args: None,
    }
}

pub(crate) fn warn_args(
    key: &str,
    file: &str,
    problem: &str,
    used: &str,
    args: Value,
) -> ConfigNotice {
    ConfigNotice {
        key: key.to_string(),
        file: file.to_string(),
        problem: problem.to_string(),
        used: used.to_string(),
        args: Some(args),
    }
}
