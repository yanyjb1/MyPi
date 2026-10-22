//! Which blocks a resize lays out now, and which wait for the sight line.

use super::local::Local;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ResizePlan {
    pub visible: Vec<i64>,
    pub lazy: Vec<i64>,
}

/// Keep the blocks around the seam immediate; the rest are lazy.
pub fn plan(local: &Local, anchor: i64, held: Vec<i64>, margin: i64) -> ResizePlan {
    let mut plan = ResizePlan::default();
    for num in held {
        if local.at(num).is_none() {
            continue;
        }
        if (num - anchor).abs() <= margin {
            plan.visible.push(num);
        } else {
            plan.lazy.push(num);
        }
    }
    plan
}
