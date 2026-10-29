// The loop's own wake sink: kairos wakes carry nothing outward.

use crate::server::agent::tools::jobs::{Wake, WakeSink};

pub(crate) struct NoWake;

impl WakeSink for NoWake {
    fn wake(&self, _wake: Wake) {}
}
