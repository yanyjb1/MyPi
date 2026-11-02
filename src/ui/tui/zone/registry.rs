//! Every major zone, in `ZoneId` order.

use super::contract::Zone;
use super::{MainZone, ResumeZone, TreeZone};

pub fn zones() -> Vec<Box<dyn Zone>> {
    vec![
        Box::new(MainZone::default()),
        Box::new(ResumeZone::default()),
        Box::new(TreeZone::default()),
    ]
}
