//! Everything the terminal front end reads out of the server's configuration.

/// Palette overrides layered over the bundled JSON theme.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ThemeConfig {
    pub name: Option<String>,
    pub accent: Option<String>,
    pub gold: Option<String>,
    pub black: Option<String>,
    pub muted: Option<String>,
}

/// How the task list folds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TodoFold {
    On(bool),
    Capped(usize),
}

impl TodoFold {
    pub const DEFAULT_CAP: usize = 8;

    pub fn cap(self) -> usize {
        match self {
            TodoFold::On(true) => Self::DEFAULT_CAP,
            TodoFold::On(false) => usize::MAX,
            TodoFold::Capped(0) => usize::MAX,
            TodoFold::Capped(n) => n,
        }
    }
}

impl Default for TodoFold {
    fn default() -> Self {
        TodoFold::On(false)
    }
}

/// Horizontal position of the usage panel.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UsageAlign {
    #[default]
    Center,
    Left,
    Right,
}
