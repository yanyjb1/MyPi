//! The display language; the config answers, `en` until it does.

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Lang {
    #[default]
    En,
    Zh,
}

impl Lang {
    // The configuration key that answers the display language.
    pub const KEY: &str = "config.ui.lang";
    /// The config's spelling, `en` or `zh`; anything else leaves the choice alone.
    pub fn parse(name: &str) -> Option<Self> {
        match name.trim().to_ascii_lowercase().as_str() {
            "en" => Some(Self::En),
            "zh" => Some(Self::Zh),
            _ => None,
        }
    }

    // The config's own spelling, which i18n's tables are keyed by.
    pub const fn code(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::Zh => "zh",
        }
    }
}
