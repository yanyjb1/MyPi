//! Key routing and the exit requests that become handovers.

use super::MainZone;
use super::input::ExitRequest;
use super::keys;
use crate::ui::tui::action::Action;
use crate::ui::tui::zone::{Ctx, Handover, Request, Seat, SubZone, ZoneId};

pub fn dispatch(zone: &mut MainZone, a: &Action, ctx: &Ctx) -> Option<Handover> {
    let seat = keys::resolve(a, &zone.keys)?;
    match seat {
        Seat::History => {
            zone.history.deliver_action(a, ctx);
        }
        Seat::Input => {
            zone.input.deliver_action(a, ctx);
            if let Some(who) = zone.input.pending_hire.take() {
                zone.reserved.hire(who, ctx);
            }
            let text = zone.input.editor.text();
            let cursor = zone.input.editor.cursor();
            zone.reserved.text(&text, cursor);
            if let Some(reply) = zone.reserved.take_reply() {
                zone.input.apply_reply(reply);
            }
        }
        Seat::Reserved => {
            zone.reserved.deliver_action(a, ctx);
            if let Some(reply) = zone.reserved.take_reply() {
                zone.input.apply_reply(reply);
            }
        }
    }
    exits(zone)
}

/// Turn the input zone's exits into session requests or one handover.
fn exits(zone: &mut MainZone) -> Option<Handover> {
    let mut requests = std::mem::take(&mut zone.pending_requests);
    requests.extend(zone.history.take_requests());
    requests.extend(zone.reserved.take_requests());
    requests.extend(zone.input.take_requests());
    let mut exits = std::mem::take(&mut zone.pending_exits);
    exits.append(&mut zone.input.pending_exits);

    let mut to = None;
    for exit in exits {
        match exit {
            ExitRequest::Submit(text) => requests.push(Request::Submit(text)),
            ExitRequest::Interrupt => requests.push(Request::Interrupt),
            ExitRequest::OpenTree => to = Some(Some(ZoneId::Tree)),
            ExitRequest::OpenResume => to = Some(Some(ZoneId::Resume)),
            ExitRequest::Quit => to = Some(None),
            ExitRequest::New => {
                zone.history.reset();
                zone.input.clear();
                requests.push(Request::Detach);
            }
        }
    }

    match to {
        Some(to) => Some(Handover { to, requests }),
        None => {
            zone.pending_requests.extend(requests);
            None
        }
    }
}
