// The words a front end and the api both use: a model address, a compaction budget, an effort, and which wire names one.

// A model address: the provider and the id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModelId {
    pub provider: String,
    pub model: String,
}

// How much of the history a compaction keeps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Retention {
    // Characters of the tail.
    Chars(u64),

    // Newest blocks.
    Blocks(u64),
}

// How hard the session asks the model to think. Memory-only: the server never stores it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Effort {
    // Ask for nothing and take the host's default; no effort field rides the model body.
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
    // The ladder in the order Ctrl+Tab cycles through it; a model declares which rungs it takes.
    pub const LADDER: [Self; 8] = [
        Self::Auto,
        Self::None,
        Self::Minimal,
        Self::Low,
        Self::Medium,
        Self::High,
        Self::Xhigh,
        Self::Max,
    ];

    // The one spelling the server takes.
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
        Self::LADDER
            .into_iter()
            .find(|effort| effort.as_str() == raw.as_str())
    }

    // The next rung this model declares, wrapping back to `Auto`; `Auto` is always offered.
    pub fn next(self, declared: &[Self]) -> Self {
        let at = Self::LADDER
            .iter()
            .position(|rung| *rung == self)
            .expect("every rung is on the ladder");
        for step in 1..=Self::LADDER.len() {
            let rung = Self::LADDER[(at + step) % Self::LADDER.len()];
            if rung == Self::Auto || declared.contains(&rung) {
                return rung;
            }
        }
        Self::Auto
    }
}
