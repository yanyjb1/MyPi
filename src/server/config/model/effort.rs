// How hard a session asks the model to think; front ends spell it, the config file carries it.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Effort {
    // Ask for nothing and take the host's default. Never sent: the field stays off the body.
    #[default]
    Auto,
    None,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl Effort {
    // Every rung, `Auto` first, so parsing and the one list agree on a set.
    const ALL: [Self; 8] = [
        Self::Auto,
        Self::None,
        Self::Minimal,
        Self::Low,
        Self::Medium,
        Self::High,
        Self::Xhigh,
        Self::Max,
    ];

    // The one spelling the wire and every front end share.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::None => "none",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }

    pub fn parse(raw: &str) -> Option<Self> {
        let raw = raw.trim().to_ascii_lowercase();
        Self::ALL
            .into_iter()
            .find(|effort| effort.as_str() == raw.as_str())
    }
}
