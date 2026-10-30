//! What one session talks to.

/// The resolved provider/model pair. The request half expands the endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Transport {
    pub provider: String,
    pub model: String,
}

impl Transport {
    pub fn new(provider: impl Into<String>, model: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            model: model.into(),
        }
    }
}
