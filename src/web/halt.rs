// How this domain says the turn was stopped, so no layer has to read an error's text.

use std::fmt;

// The user stopped the turn: hand back what is already produced, and do nothing more.
#[derive(Debug)]
pub(crate) struct Stopped;

impl fmt::Display for Stopped {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("the call was stopped")
    }
}

impl std::error::Error for Stopped {}

// Whether this error is the stop and nothing else.
pub(crate) fn is(e: &anyhow::Error) -> bool {
    e.downcast_ref::<Stopped>().is_some()
}
