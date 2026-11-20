//! What every zone shares with the front end: one post box, one color.

use crate::ui::tui::theme::Palette;

use super::mail::Mail;
use super::outcome::ZoneId;

/// The whole surface a zone is handed at boot. Nothing here reaches the server by itself.
#[derive(Debug, Clone, Default)]
pub struct Shared {
    pub mail: Mail,
    pub palette: Palette,
}

impl Shared {
    /// The same box and the same color, with every letter stamped to `from`.
    pub fn at(&self, from: ZoneId) -> Shared {
        Shared {
            mail: self.mail.at(from),
            palette: self.palette.clone(),
        }
    }
}
