//! Interrupt: set the running round's stop flag.

use crate::server::api::driver::Driver;

pub(crate) fn apply(driver: &Driver, session: i64) {
    driver.interrupt(session);
}
