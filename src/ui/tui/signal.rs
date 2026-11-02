//! Process stop signals: the one fact the loop cannot read from the terminal.
//!
//! `SIGINT`, `SIGTERM`, `SIGQUIT` and `SIGHUP` end the front end. The loop restores the screen
//! and leaves; nothing tries to survive them.

use signal_hook::consts::{SIGHUP, SIGINT, SIGQUIT, SIGTERM};
use signal_hook::iterator::Signals;

/// The stop signals, registered once at startup.
pub struct Stop {
    signals: Signals,
}

impl Stop {
    pub fn new() -> std::io::Result<Stop> {
        Ok(Stop {
            signals: Signals::new([SIGINT, SIGTERM, SIGQUIT, SIGHUP])?,
        })
    }

    /// The first stop signal seen since the last call.
    pub fn poll(&mut self) -> Option<i32> {
        self.signals.pending().next()
    }
}
