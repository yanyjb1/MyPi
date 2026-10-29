//! The server's own line about one session.

use super::relay::Relay;

pub(crate) fn notify(relay: &Relay, session: i64, level: &str, content: &str) {
    relay.notice(session, level, content);
}
