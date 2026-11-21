// The app's own words for a major zone.

use std::time::Duration;

use ratatui::text::Line;

use super::claim::Claim;
use super::kind::Kind;
use super::local::Local;
use super::memo::Memo;
use super::outcome::Outcome;
use super::shared::Shared;
use super::size::TermSize;
use crate::ipc::contract::Frame;
use crate::ui::tui::action::Action;

// A major zone. The app only ever sees this.
pub trait Zone {
    // The post box and the palette: handed once, at boot.
    fn bind(&mut self, shared: Shared);

    // The draft's directory moved with the conversation; only the main zone keeps one.
    fn draft_moved(&mut self, _cwd: &str) {}

    // The draft became a row: its extra fields are spent.
    fn draft_cleared(&mut self) {}

    // The directory a new session would open in, if this zone keeps a draft.
    fn draft_cwd(&self) -> Option<String> {
        None
    }

    // Every input this zone consumes, declared once.
    fn keys(&self) -> Vec<Claim>;

    // The frame kinds this zone handles. `local` never appears here: it is unconditional.
    fn kinds(&self) -> &'static [Kind];

    // The app gives the screen to this zone: the size, and the memo the last zone left.
    fn start(&mut self, size: TermSize, memo: &Memo) -> Vec<Local>;

    // The conversation was entered (`Some`) or left (`None`).
    fn session(&mut self, id: Option<i64>) -> Vec<Local>;

    // One frame the zone declared (or any `local` one).
    fn event(&mut self, frame: &Frame) -> Vec<Local>;

    // One input, already resolved to this zone.
    fn action(&mut self, action: &Action) -> (Outcome, Vec<Local>);

    // One frame's side effects, before any drawing: the asks this zone owes.
    fn tick(&mut self) -> Vec<Local> {
        Vec::new()
    }

    // Bumped whenever this zone's rows would differ.
    fn revision(&self) -> u64;

    // How often time alone rewrites the rows; `None` = never.
    fn anim(&self) -> Option<Duration> {
        None
    }

    // This zone's rows for the frame.
    fn render(&mut self) -> Vec<Line<'static>>;

    // Where the hardware cursor goes, in terminal coordinates.
    fn cursor(&self) -> Option<(u16, u16)> {
        None
    }
}
