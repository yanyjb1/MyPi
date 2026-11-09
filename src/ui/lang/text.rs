//! One sentence written twice, picked when it is drawn.

use super::Lang;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Text {
    pub en: &'static str,
    pub zh: &'static str,
}

impl Text {
    /// The spelling this language reads.
    pub const fn get(self, lang: Lang) -> &'static str {
        match lang {
            Lang::En => self.en,
            Lang::Zh => self.zh,
        }
    }
}
