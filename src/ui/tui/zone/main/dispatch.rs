//! Key routing, and the exits that become requests or one handover.

use serde_json::json;

use super::input::ExitRequest;
use super::keys::{self, Seat};
use super::zone::MainZone;
use crate::ui::tui::action::Action;
use crate::ui::tui::zone::{Outcome, Request, SubZone, ZoneId};

pub fn dispatch(zone: &mut MainZone, a: &Action) -> (Outcome, Vec<Request>) {
    let Some(seat) = keys::resolve(a, &zone.keys) else {
        return (Outcome::Stay, Vec::new());
    };
    match seat {
        Seat::History => {
            zone.history.deliver_action(a);
        }
        Seat::Input => {
            zone.input.deliver_action(a);
        }
        Seat::Reserved => {
            zone.reserved.deliver_action(a);
        }
    }
    exits(zone)
}

/// Turn the input zone's exits into session requests or one handover.
fn exits(zone: &mut MainZone) -> (Outcome, Vec<Request>) {
    let mut requests = Vec::new();
    let mut outcome = Outcome::Stay;
    for exit in zone.input.take_exits() {
        match exit {
            ExitRequest::Submit(text) => {
                requests.push(Request::subscribe("submit", json!({"text": text})));
            }
            ExitRequest::Interrupt => requests.push(Request::post("interrupt", json!({}))),
            ExitRequest::OpenTree => {
                outcome = Outcome::Show {
                    to: ZoneId::Tree,
                    memo: Vec::new(),
                };
            }
            ExitRequest::OpenResume => {
                outcome = Outcome::Show {
                    to: ZoneId::Resume,
                    memo: Vec::new(),
                };
            }
            ExitRequest::Quit => outcome = Outcome::Quit,
            ExitRequest::New => {
                zone.history.reset();
                zone.input.clear();
                requests.push(Request::post("mailbox", json!({})));
                requests.push(Request::Detach);
            }
        }
    }
    (outcome, requests)
}
