//! Configuration warnings: a key that was written but not used.

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigNotice {
    pub key: String,
    pub file: String,
    pub problem: String,
    pub used: String,
}

pub(crate) fn warn(key: &str, file: &str, problem: &str, used: &str) -> ConfigNotice {
    ConfigNotice {
        key: key.to_string(),
        file: file.to_string(),
        problem: problem.to_string(),
        used: used.to_string(),
    }
}
