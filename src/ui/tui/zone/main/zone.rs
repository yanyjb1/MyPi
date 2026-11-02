//! The chat zone: the host of history, input and reserved.

use serde_json::json;

use super::{arbitrate, dispatch, history, input, keys, reserved};
use crate::ui::tui::zone::{Claim, Kind, Memo, Outcome, Packet, Request, SubZone, TermSize, Zone};

/// The chat zone. Sub-zones are private; their behaviour lives in their own modules.
#[derive(Default)]
pub struct MainZone {
    pub(super) history: history::zone::HistoryZone,
    pub(super) input: input::InputZone,
    pub(super) reserved: reserved::ReservedZone,
    pub(super) keys: keys::Borrowed,
    pub(super) size: Option<TermSize>,
    pub(super) heights: super::arbitrate::Heights,
}

impl MainZone {
    /// Where the hardware cursor goes, in terminal coordinates.
    fn cursor_position(&self) -> Option<(u16, u16)> {
        super::arbitrate::cursor_position(self)
    }
}

impl Zone for MainZone {
    fn name(&self) -> &'static str {
        "main"
    }

    fn keys(&self) -> Vec<Claim> {
        let mut out = Vec::new();
        out.extend_from_slice(history::zone::CLAIMS);
        out.extend_from_slice(input::InputZone::CLAIMS);
        out.extend_from_slice(reserved::ReservedZone::CLAIMS);
        out
    }

    fn kinds(&self) -> &'static [Kind] {
        &[Kind::Stream, Kind::Call, Kind::Notice]
    }

    fn start(&mut self, size: TermSize, memo: &Memo) -> Vec<Request> {
        self.size = Some(size);
        if let Some(focus) = memo.read("tree.focus") {
            memo.delete("tree.focus");
            if let Some((turn, block)) = focus.split_once(':')
                && let (Ok(turn), Ok(block)) = (turn.parse::<i64>(), block.parse::<i64>())
            {
                self.history.focus(turn, block);
            }
        }
        vec![Request::Structure, Request::call("init", json!({}))]
    }

    fn session(&mut self, id: Option<i64>) -> Vec<Request> {
        match id {
            Some(id) => {
                self.history.attach(id);
                self.input.attach(id);
                self.reserved.attach(id);
            }
            None => {
                self.history.detach();
                self.input.detach();
                self.reserved.detach();
                self.keys.clear();
            }
        }
        Vec::new()
    }

    fn event(&mut self, packet: &Packet) -> Vec<Request> {
        if packet.name == "resized"
            && let Some(size) = TermSize::of(&packet.content)
        {
            self.size = Some(size);
        }
        self.lend();
        let kind = Kind::of(&packet.kind, &packet.name);
        let size = self.size.unwrap_or(TermSize { cols: 80, rows: 24 });
        let mut requests = Vec::new();
        for sub in [
            &mut self.history as &mut dyn SubZone,
            &mut self.input,
            &mut self.reserved,
        ] {
            if kind.is_some_and(|kind| sub.kinds().contains(&kind)) || packet.kind == "local" {
                requests.extend(sub.deliver_event(packet, size));
            }
        }
        requests
    }

    fn action(&mut self, action: &crate::ui::tui::action::Action) -> (Outcome, Vec<Request>) {
        self.lend();
        dispatch::dispatch(self, action)
    }

    fn structure(&mut self, snapshot: &crate::ui::session::Structure) -> Vec<Request> {
        self.history.structure(snapshot);
        self.input.structure(snapshot);
        self.reserved.structure(snapshot);
        Vec::new()
    }

    fn render(&mut self) -> Vec<ratatui::text::Line<'static>> {
        self.settle();
        let palette = crate::ui::tui::theme::Palette::current();
        let theme = crate::ui::tui::theme::HistoryTheme::resolve();
        let width = self.size.map(|size| size.cols).unwrap_or(80);
        let mut out = self.history.render_rows(width, &theme);
        for _ in 0..self.heights.gap {
            out.push(ratatui::text::Line::default());
        }
        out.extend(self.input.render_rows(width, &palette));
        self.reserved.col(self.input.cursor_col);
        out.extend(self.reserved.render_rows(width));
        out
    }

    fn cursor(&self) -> Option<(u16, u16)> {
        self.cursor_position()
    }
}

impl MainZone {
    fn settle(&mut self) {
        arbitrate::settle(self);
    }

    fn lend(&mut self) {
        arbitrate::lend(self);
    }
}
